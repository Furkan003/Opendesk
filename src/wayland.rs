//! Wayland portal integration. The compositor owns screen/input consent.
//! Runtime dependencies: xdg-desktop-portal backend, PipeWire and GStreamer's
//! pipewiresrc/videoconvert/jpegenc/fdsink plugins. No private compositor bypass.
use ashpd::desktop::{
    PersistMode,
    remote_desktop::{DeviceType, KeyState, RemoteDesktop},
    screencast::{CursorMode, Screencast, SourceType},
};
use opendesk::network::InputEvent;
use std::{
    io::Read,
    os::{fd::AsRawFd, unix::process::CommandExt},
    process::{Command, Stdio},
    sync::{Arc, Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};

type Frame = Arc<Mutex<Option<(Instant, Result<Vec<u8>, String>)>>>;
enum Request {
    Input(InputEvent),
    Stop,
}
struct Portal {
    tx: tokio::sync::mpsc::UnboundedSender<Request>,
    frame: Frame,
    width: u32,
    height: u32,
}
static PORTAL: OnceLock<Mutex<Option<Portal>>> = OnceLock::new();
static INIT_LOCK: Mutex<()> = Mutex::new(());
static INPUT_ERROR: Mutex<Option<String>> = Mutex::new(None);
static INIT_ERROR: Mutex<Option<(u64, String)>> = Mutex::new(None);
static EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct CaptureChild(std::process::Child);
impl Drop for CaptureChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
static STOPPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn start() {
    EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    STOPPED.store(false, std::sync::atomic::Ordering::SeqCst);
}
pub fn active() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|v| v == "wayland")
}
pub fn stop() {
    EPOCH.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    STOPPED.store(true, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut state) = PORTAL.get_or_init(|| Mutex::new(None)).lock() {
        if let Some(p) = state.take() {
            let _ = p.tx.send(Request::Stop);
        }
    }
}
/// Opens the OS dialog only on a background worker. Capture callers wait for
/// that same initialization; GUI callers use prepare_async and never wait.
pub fn prepare_async() {
    std::thread::spawn(|| {
        let _ = ensure();
    });
}
fn ensure() -> Result<(), String> {
    let epoch = EPOCH.load(std::sync::atomic::Ordering::SeqCst);
    let _initialization = loop {
        if STOPPED.load(std::sync::atomic::Ordering::SeqCst)
            || EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch
        {
            return Err("Wayland sharing stopped".into());
        }
        match INIT_LOCK.try_lock() {
            Ok(guard) => break guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(_) => return Err("Wayland initialization lock failed".into()),
        }
    };
    let state = PORTAL
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "Wayland session lock failed")?;
    if state.is_some() {
        return Ok(());
    }
    drop(state);
    if let Ok(error) = INIT_ERROR.lock() {
        if let Some((failed_epoch, message)) = error.as_ref() {
            if *failed_epoch == epoch {
                return Err(message.clone());
            }
        }
    }
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let frame: Frame = Arc::new(Mutex::new(None));
    let worker_frame = frame.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(r) => r,
            Err(e) => {
                let _ = ready_tx.send(Err(e.to_string()));
                return;
            }
        };
        let result:Result<(),String>=rt.block_on(async {
            let remote=RemoteDesktop::new().await.map_err(|e|format!("Wayland RemoteDesktop portal unavailable: {e}"))?;
            let screen=Screencast::new().await.map_err(|e|e.to_string())?;
            let session=remote.create_session().await.map_err(|e|e.to_string())?;
            remote.select_devices(&session,DeviceType::Keyboard|DeviceType::Pointer,None,PersistMode::DoNot).await.map_err(|e|e.to_string())?;
            screen.select_sources(&session,CursorMode::Embedded,SourceType::Monitor.into(),false,None,PersistMode::DoNot).await.map_err(|e|e.to_string())?;
            let start_request=remote.start(&session,None);
            tokio::pin!(start_request);
            let request=loop { tokio::select! {
                response=&mut start_request=>break response.map_err(|e|e.to_string())?,
                request=rx.recv()=>match request {
                    Some(Request::Stop)|None=>{let _=session.close().await;return Err("Wayland sharing stopped".into());},
                    Some(Request::Input(_))=>{if let Ok(mut last)=INPUT_ERROR.lock(){*last=Some("Awaiting Wayland sharing approval".into());}},
                }
            }};
            let response=request.response().map_err(|e|format!("Wayland sharing was not approved: {e}"))?;
            let stream=response.streams().and_then(|s|s.first()).ok_or("Portal returned no selected monitor")?;
            let node=stream.pipe_wire_node_id();let (width,height)=stream.size().ok_or("Portal did not provide monitor dimensions")?;
            if width<=0||height<=0{return Err("Portal returned invalid monitor dimensions".into());}
            let fd=screen.open_pipe_wire_remote(&session).await.map_err(|e|e.to_string())?;let raw_fd=fd.as_raw_fd();
            let mut command=Command::new("gst-launch-1.0");command.args(["-q","pipewiresrc",&format!("fd={raw_fd}"),&format!("path={node}"),"do-timestamp=true","!","videoconvert","!","jpegenc","quality=80","!","fdsink","fd=1"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
            // Only the portal-supplied remote descriptor is inherited by this
            // owned child; do not clear CLOEXEC in the parent process.
            unsafe {command.pre_exec(move||{let flags=libc::fcntl(raw_fd,libc::F_GETFD);if flags<0||libc::fcntl(raw_fd,libc::F_SETFD,flags&!libc::FD_CLOEXEC)<0{return Err(std::io::Error::last_os_error());}Ok(())});}
            let mut child=CaptureChild(command.spawn().map_err(|e|format!("Wayland capture needs gst-launch-1.0 with PipeWire and JPEG plugins: {e}"))?);
            let mut output=child.0.stdout.take().ok_or("Wayland capture child has no stdout")?;
            let read_frame=worker_frame.clone();std::thread::spawn(move||{let mut bytes=[0u8;16384];let mut pending=Vec::new();loop {match output.read(&mut bytes){Ok(0)=>break,Ok(n)=>{pending.extend_from_slice(&bytes[..n]);while let Some(start)=pending.windows(2).position(|b|b==[0xff,0xd8]){if start>0{pending.drain(..start);}let Some(end)=pending.windows(2).skip(1).position(|b|b==[0xff,0xd9]).map(|i|i+3) else {break;};let jpeg:Vec<_>=pending.drain(..end).collect();if let Ok(mut frame)=read_frame.lock(){*frame=Some((Instant::now(),Ok(jpeg)));}}if pending.len()>16*1024*1024{break;}},Err(_)=>break,}}if let Ok(mut frame)=read_frame.lock(){*frame=Some((Instant::now(),Err("Wayland video stream ended; check portal permission and GStreamer plugins".into())));}});
            let _=ready_tx.send(Ok((width as u32,height as u32)));
            let mut interval=tokio::time::interval(Duration::from_millis(200));
            loop {tokio::select! {
                request=rx.recv()=>match request {Some(Request::Stop)|None=>break,Some(Request::Input(event))=>{if let Err(error)=apply(&remote,&session,node,width,height,event).await { if let Ok(mut last)=INPUT_ERROR.lock() { *last=Some(error); } }}},
                _=interval.tick()=>{if child.0.try_wait().map_err(|e|e.to_string())?.is_some(){break;}}
            }}
            let _=child.0.kill();let _=child.0.wait();let _=session.close().await;Ok(())
        });
        if let Err(e) = result {
            let _ = ready_tx.send(Err(e.clone()));
            if let Ok(mut f) = worker_frame.lock() {
                *f = Some((Instant::now(), Err(e)));
            }
        }
    });
    let (width, height) = loop {
        if STOPPED.load(std::sync::atomic::Ordering::SeqCst)
            || EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch
        {
            let _ = tx.send(Request::Stop);
            return Err("Wayland sharing stopped".into());
        }
        match ready_rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(size)) => break size,
            Ok(Err(message)) => {
                if let Ok(mut error) = INIT_ERROR.lock() {
                    *error = Some((epoch, message.clone()));
                }
                return Err(message);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => return Err("Wayland sharing setup ended".into()),
        }
    };
    let mut state = PORTAL
        .get()
        .unwrap()
        .lock()
        .map_err(|_| "Wayland session lock failed")?;
    if STOPPED.load(std::sync::atomic::Ordering::SeqCst)
        || EPOCH.load(std::sync::atomic::Ordering::SeqCst) != epoch
    {
        let _ = tx.send(Request::Stop);
        return Err("Wayland sharing stopped".into());
    }
    *state = Some(Portal {
        tx,
        frame,
        width,
        height,
    });
    Ok(())
}
pub fn capture() -> Result<Vec<u8>, String> {
    ensure()?;
    let frame = {
        let state = PORTAL
            .get()
            .ok_or("Awaiting Wayland sharing approval")?
            .lock()
            .map_err(|_| "Wayland session unavailable")?;
        state
            .as_ref()
            .ok_or("Wayland session unavailable")?
            .frame
            .clone()
    };
    for _ in 0..60 {
        if let Some((at, result)) = frame
            .lock()
            .map_err(|_| "Wayland frame unavailable")?
            .as_ref()
        {
            if at.elapsed() > Duration::from_secs(3) {
                return Err("Wayland capture stopped producing frames".into());
            }
            return result.clone();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("Wayland capture produced no frame; install GStreamer PipeWire/JPEG plugins".into())
}
pub fn input(event: InputEvent) -> Result<(), String> {
    let tx = {
        let state = PORTAL
            .get()
            .ok_or("Awaiting Wayland sharing approval")?
            .lock()
            .map_err(|_| "Wayland session unavailable")?;
        state
            .as_ref()
            .ok_or("Wayland session unavailable")?
            .tx
            .clone()
    };
    if let Ok(mut last) = INPUT_ERROR.lock() {
        if let Some(error) = last.take() {
            return Err(error);
        }
    }
    // DBus processing stays on the portal worker, including delayed replies.
    tx.send(Request::Input(event))
        .map_err(|_| "Wayland session ended".to_owned())
}
async fn apply(
    remote: &RemoteDesktop<'_>,
    session: &ashpd::desktop::Session<'_, RemoteDesktop<'_>>,
    node: u32,
    width: i32,
    height: i32,
    event: InputEvent,
) -> Result<(), String> {
    let state = |pressed| {
        if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        }
    };
    let result = match event {
        InputEvent::Move { x, y } => {
            if !x.is_finite() || !y.is_finite() {
                return Err("Invalid pointer coordinate".into());
            }
            remote
                .notify_pointer_motion_absolute(
                    session,
                    node,
                    x.clamp(0.0, 1.0) as f64 * (width - 1) as f64,
                    y.clamp(0.0, 1.0) as f64 * (height - 1) as f64,
                )
                .await
        }
        InputEvent::Button { button, pressed } => {
            remote
                .notify_pointer_button(
                    session,
                    match button {
                        0 => 272,
                        1 => 273,
                        2 => 274,
                        _ => return Err("Unsupported pointer button".into()),
                    },
                    state(pressed),
                )
                .await
        }
        InputEvent::Scroll { x, y } => {
            remote
                .notify_pointer_axis(session, x as f64 * 15.0, y as f64 * 15.0, true)
                .await
        }
        InputEvent::Key { key, pressed } => {
            remote
                .notify_keyboard_keysym(
                    session,
                    keysym(&key).ok_or("Unsupported Wayland key")? as i32,
                    state(pressed),
                )
                .await
        }
        InputEvent::Text(text) => {
            for c in text.chars() {
                let key = if c as u32 <= 0xff {
                    c as u32
                } else {
                    0x01000000 | c as u32
                };
                remote
                    .notify_keyboard_keysym(session, key as i32, KeyState::Pressed)
                    .await
                    .map_err(|e| e.to_string())?;
                remote
                    .notify_keyboard_keysym(session, key as i32, KeyState::Released)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            return Ok(());
        }
        InputEvent::ReleaseAll => return Ok(()),
    };
    result.map_err(|e| e.to_string())
}
fn keysym(key: &str) -> Option<u32> {
    Some(match key {
        "Enter" => 0xff0d,
        "Escape" => 0xff1b,
        "Tab" => 0xff09,
        "Backspace" => 0xff08,
        "Delete" => 0xffff,
        "Insert" => 0xff63,
        "ArrowUp" => 0xff52,
        "ArrowDown" => 0xff54,
        "ArrowLeft" => 0xff51,
        "ArrowRight" => 0xff53,
        "Home" => 0xff50,
        "End" => 0xff57,
        "PageUp" => 0xff55,
        "PageDown" => 0xff56,
        "Control" | "Ctrl" => 0xffe3,
        "Shift" => 0xffe1,
        "Alt" => 0xffe9,
        "Meta" | "Super" | "Command" => 0xffeb,
        "CapsLock" => 0xffe5,
        "Space" => 32,
        "Num0" => 48,
        "Num1" => 49,
        "Num2" => 50,
        "Num3" => 51,
        "Num4" => 52,
        "Num5" => 53,
        "Num6" => 54,
        "Num7" => 55,
        "Num8" => 56,
        "Num9" => 57,
        "Minus" => 45,
        "Plus" | "Equals" => 61,
        "Comma" => 44,
        "Period" => 46,
        "Slash" => 47,
        "Backslash" => 92,
        "Semicolon" | "Colon" => 59,
        "Quote" => 39,
        "Backtick" => 96,
        "OpenBracket" => 91,
        "CloseBracket" => 93,
        _ => {
            if let Some(n) = key
                .strip_prefix('F')
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|n| (1..=20).contains(n))
            {
                return Some(0xffbd + n);
            }
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            if c as u32 <= 0xff {
                c.to_ascii_lowercase() as u32
            } else {
                0x01000000 | c as u32
            }
        }
    })
}
pub fn current_dimensions() -> Option<(u32, u32)> {
    let state = PORTAL.get()?.lock().ok()?;
    state.as_ref().map(|p| (p.width, p.height))
}
