//! Encrypted, explicitly approved, single-peer LAN sessions.
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Receiver, SyncSender, TryRecvError},
    thread,
    time::Duration,
};

const MAX_FRAME: usize = 12 * 1024 * 1024;
const MAX_FILE: usize = 16 * 1024 * 1024;
const CHUNK: usize = 12 * 1024;
const COMMANDS_PER_TURN: usize = 32;
/// Event queue towards the UI; deep enough that a busy or minimized window
/// never closes the session because a bounded queue filled up.
const EVENT_QUEUE: usize = 256;
const COMMAND_QUEUE: usize = 64;
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Permissions {
    pub screen: bool,
    pub input: bool,
    pub clipboard: bool,
    pub files: bool,
    pub audio: bool,
}
impl Default for Permissions {
    fn default() -> Self {
        Self {
            screen: true,
            input: false,
            clipboard: true,
            files: true,
            audio: false,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputEvent {
    ReleaseAll,
    Move { x: f32, y: f32 },
    Button { button: u8, pressed: bool },
    Key { key: String, pressed: bool },
    Text(String),
    Scroll { x: i32, y: i32 },
}
#[derive(Serialize, Deserialize)]
pub enum Event {
    TransferOffer(crate::file_protocol::Manifest),
    TransferProgress {
        id: String,
        completed: u64,
        total: u64,
        receiving: bool,
    },
    TransferComplete {
        id: String,
    },
    Authentication {
        password: String,
        totp: String,
    },
    Permissions(Permissions),
    Monitors(Vec<MonitorInfo>),
    SelectMonitor(usize),
    VideoFeedback {
        max_fps: u8,
        quality: u8,
    },
    Metrics {
        sent_bytes: u64,
        received_bytes: u64,
    },
    Audio {
        sample_rate: u32,
        channels: u8,
        pcm: Vec<i16>,
    },
    Clipboard(String),
    IncomingFile {
        name: String,
        bytes: Vec<u8>,
    },
    Chat(String),
    Status(String),
    Incoming {
        peer: String,
    },
    Connected {
        peer: String,
        control: bool,
    },
    Frame {
        jpeg: Vec<u8>,
    },
    Input(InputEvent),
    Disconnected,
}
#[derive(Serialize, Deserialize)]
pub enum Command {
    SendPath(std::path::PathBuf),
    ReceiveTransfer {
        directory: std::path::PathBuf,
    },
    RejectTransfer,
    Authenticate {
        password: String,
        totp: String,
    },
    AcceptPermissions(Permissions),
    SetPermissions(Permissions),
    Monitors(Vec<MonitorInfo>),
    SelectMonitor(usize),
    VideoFeedback {
        max_fps: u8,
        quality: u8,
    },
    Audio {
        sample_rate: u32,
        channels: u8,
        pcm: Vec<i16>,
    },
    Clipboard(String),
    SendFile {
        name: String,
        bytes: Vec<u8>,
    },
    Chat(String),
    Accept {
        control: bool,
    },
    Reject,
    Frame(Vec<u8>),
    Input(InputEvent),
    Stop,
}
pub struct Connection {
    pub events: Receiver<Event>,
    pub commands: SyncSender<Command>,
    /// Keeps a peer-to-peer tunnel alive; dropping the connection closes it.
    pub(crate) tunnel: Option<crate::p2p::Guard>,
}
#[derive(Serialize, Deserialize)]
enum Wire {
    Ping,
    TransferOfferStart(usize),
    TransferManifestChunk(Vec<u8>),
    TransferResume(Vec<crate::file_protocol::ResumeRequest>),
    TransferChunk {
        index: usize,
        offset: u64,
        bytes: Vec<u8>,
        finished: bool,
    },
    TransferFinished(String),
    TransferRejected,
    Authenticate {
        password: String,
        totp: String,
    },
    Hello {
        version: u16,
        capabilities: u32,
    },
    Permissions(Permissions),
    Monitors(Vec<MonitorInfo>),
    SelectMonitor(usize),
    VideoFeedback {
        max_fps: u8,
        quality: u8,
    },
    Audio {
        sample_rate: u32,
        channels: u8,
        pcm: Vec<i16>,
    },
    Clipboard(String),
    FileStart {
        name: String,
        size: usize,
    },
    FileChunk(Vec<u8>),
    Chat(String),
    Accepted(Permissions),
    Rejected,
    FrameStart(usize),
    Chunk(Vec<u8>),
    Input(InputEvent),
    Bye,
}
pub fn generate_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
/// The address this computer uses on its local network, if it has a route.
/// Connecting a UDP socket only selects a route; no packet is sent.
pub fn local_ip() -> Option<std::net::IpAddr> {
    ["8.8.8.8:80", "10.255.255.255:1", "192.168.255.255:1"]
        .iter()
        .find_map(|probe| {
            let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
            socket.connect(probe).ok()?;
            let ip = socket.local_addr().ok()?.ip();
            (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
        })
}
/// Adds the default Opendesk port when a viewer types only an IP or host name.
pub fn with_default_port(address: &str) -> String {
    let address = address.trim();
    if address.is_empty()
        || address.parse::<std::net::SocketAddr>().is_ok()
        || (address.contains(':') && !address.contains("::") && address.matches(':').count() == 1)
    {
        address.to_owned()
    } else if address.parse::<std::net::Ipv6Addr>().is_ok() {
        format!("[{address}]:7878")
    } else {
        format!("{address}:7878")
    }
}
pub fn generate_code() -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut random = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut random);
    random
        .chunks(5)
        .map(|group| {
            group
                .iter()
                .map(|b| ALPHABET[(b & 31) as usize] as char)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}
pub fn host_relay(relay: &str, password: &str) -> Result<Connection, String> {
    relay_connection(relay, password, true)
}
pub fn connect_relay(relay: &str, password: &str) -> Result<Connection, String> {
    relay_connection(relay, password, false)
}
fn relay_connection(relay: &str, password: &str, is_host: bool) -> Result<Connection, String> {
    use std::net::ToSocketAddrs;
    let key = secret(password)?;
    let endpoint = relay
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("Relay address has no endpoint")?;
    let mut stream =
        TcpStream::connect_timeout(&endpoint, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let preamble = format!(
        "ODESK1{}{}",
        if is_host { 'H' } else { 'V' },
        crate::relay::channel_id(&key)
    );
    stream
        .write_all(preamble.as_bytes())
        .map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
    let (cmd, commands) = mpsc::sync_channel(COMMAND_QUEUE);
    thread::spawn(move || {
        let waiting = (|| -> Result<(), String> {
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .map_err(|e| e.to_string())?;
            if is_host {
                relay_ack(&mut stream, &commands, deadline, b"WAIT!\n")?;
                tx.try_send(Event::Status("Waiting at relay for viewer".into()))
                    .map_err(|_| "UI unavailable")?;
            }
            relay_ack(&mut stream, &commands, deadline, b"READY\n")?;
            Ok(())
        })();
        match waiting {
            Ok(()) => run(stream, endpoint.to_string(), is_host, key, &tx, &commands),
            Err(e) => {
                let _ = tx.try_send(Event::Status(e));
            }
        }
        let _ = tx.try_send(Event::Disconnected);
    });
    Ok(Connection {
        events: rx,
        commands: cmd,
        tunnel: None,
    })
}
fn relay_ack(
    stream: &mut TcpStream,
    commands: &Receiver<Command>,
    deadline: std::time::Instant,
    expected: &[u8; 6],
) -> Result<(), String> {
    let mut ack = [0; 6];
    let mut offset = 0;
    while offset < 6 {
        if std::time::Instant::now() >= deadline {
            return Err("Relay pairing timed out".into());
        }
        match commands.try_recv() {
            Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                return Err("Session stopped".into());
            }
            _ => {}
        }
        match stream.read(&mut ack[offset..]) {
            Ok(0) => return Err("Relay disconnected".into()),
            Ok(n) => offset += n,
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    if &ack != expected {
        return Err("Relay pairing failed".into());
    }
    Ok(())
}
pub(crate) fn secret(s: &str) -> Result<[u8; 32], String> {
    let normalized = s.replace('-', "").to_ascii_uppercase();
    if normalized.len() == 20
        && normalized
            .bytes()
            .all(|b| b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789".contains(&b))
    {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        hash.update(b"Opendesk code v2");
        hash.update(normalized.as_bytes());
        return Ok(hash.finalize().into());
    }
    if s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Use a 20-symbol Opendesk code or a 64-character hexadecimal secret".into());
    }
    let mut key = [0; 32];
    for (i, k) in key.iter_mut().enumerate() {
        *k = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(key)
}
pub fn host(bind: &str, password: &str) -> Result<Connection, String> {
    let key = secret(password)?;
    let listener = TcpListener::bind(bind).map_err(|e| e.to_string())?;
    host_listener(listener, key)
}
fn host_listener(listener: TcpListener, key: [u8; 32]) -> Result<Connection, String> {
    local_listener(listener, key, true, |peer| peer.to_string()).map(|(c, _)| c)
}
/// Accepts the session socket on `listener`. Hosts keep listening after a peer fails
/// the handshake; viewers (tunnel endpoints) accept exactly one socket. The returned
/// sender lets a tunnel report its own status into the session's event stream.
pub(crate) fn local_listener(
    listener: TcpListener,
    key: [u8; 32],
    is_host: bool,
    label: impl Fn(std::net::SocketAddr) -> String + Send + 'static,
) -> Result<(Connection, SyncSender<Event>), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
    let (cmd, commands) = mpsc::sync_channel(COMMAND_QUEUE);
    let notices = tx.clone();
    thread::spawn(move || {
        if is_host {
            let _ = tx.try_send(Event::Status("Waiting for connection".into()));
        }
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => break,
                _ => {}
            }
            match listener.accept() {
                // A peer without the pairing key (or with an old protocol) must not
                // end hosting; keep listening until an authenticated peer arrives.
                Ok((mut stream, peer)) => match establish(&mut stream, is_host, &key) {
                    Ok((noise, capabilities)) => {
                        run_established(
                            stream,
                            label(peer),
                            is_host,
                            noise,
                            capabilities,
                            &tx,
                            &commands,
                        );
                        break;
                    }
                    Err(e) if is_host => {
                        let _ = tx.try_send(Event::Status(format!(
                            "Ignored connection from {}: {e}. Still waiting",
                            label(peer)
                        )));
                    }
                    Err(e) => {
                        let _ = tx.try_send(Event::Status(e));
                        break;
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20))
                }
                Err(e) => {
                    let _ = tx.try_send(Event::Status(e.to_string()));
                    break;
                }
            }
        }
        let _ = tx.try_send(Event::Disconnected);
    });
    Ok((
        Connection {
            events: rx,
            commands: cmd,
            tunnel: None,
        },
        notices,
    ))
}
pub fn connect(addr: &str, password: &str) -> Result<Connection, String> {
    let key = secret(password)?;
    use std::net::ToSocketAddrs;
    let endpoint = addr
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("Address has no endpoint")?;
    let stream =
        TcpStream::connect_timeout(&endpoint, Duration::from_secs(3)).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::sync_channel(EVENT_QUEUE);
    let (cmd, commands) = mpsc::sync_channel(COMMAND_QUEUE);
    thread::spawn(move || {
        run(stream, endpoint.to_string(), false, key, &tx, &commands);
        let _ = tx.try_send(Event::Disconnected);
    });
    Ok(Connection {
        events: rx,
        commands: cmd,
        tunnel: None,
    })
}
fn handshake(
    stream: &mut TcpStream,
    host: bool,
    key: &[u8; 32],
) -> Result<snow::TransportState, String> {
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let builder = snow::Builder::new(
        "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s"
            .parse()
            .map_err(|e: snow::Error| e.to_string())?,
    )
    .psk(0, key);
    let mut hs = if host {
        builder.build_responder()
    } else {
        builder.build_initiator()
    }
    .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 65535];
    if !host {
        let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
        packet(stream, &buf[..n])?;
    }
    let msg = read_packet(stream)?;
    hs.read_message(&msg, &mut buf)
        .map_err(|_| "Authentication failed".to_string())?;
    if host {
        let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
        packet(stream, &buf[..n])?;
    }
    hs.into_transport_mode().map_err(|e| e.to_string())
}
pub(crate) fn packet(s: &mut TcpStream, b: &[u8]) -> Result<(), String> {
    if b.len() > 65535 {
        return Err("Packet too large".into());
    }
    s.write_all(&(b.len() as u32).to_be_bytes())
        .and_then(|_| s.write_all(b))
        .map_err(|e| e.to_string())
}
pub(crate) fn read_packet(s: &mut TcpStream) -> Result<Vec<u8>, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut h = [0; 4];
    read_until(s, &mut h, deadline)?;
    let n = u32::from_be_bytes(h) as usize;
    if !(16..=65535).contains(&n) {
        return Err("Invalid packet length".into());
    }
    let mut b = vec![0; n];
    read_until(s, &mut b, deadline)?;
    Ok(b)
}
fn read_until(
    s: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: std::time::Instant,
) -> Result<(), String> {
    while !bytes.is_empty() {
        let remaining = deadline
            .checked_duration_since(std::time::Instant::now())
            .ok_or("Handshake timed out")?;
        s.set_read_timeout(Some(remaining))
            .map_err(|e| e.to_string())?;
        let n = s.read(bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("Peer disconnected during handshake".into());
        }
        bytes = &mut bytes[n..];
    }
    Ok(())
}
fn send(s: &mut TcpStream, noise: &mut snow::TransportState, w: &Wire) -> Result<(), String> {
    let plain = serde_json::to_vec(w).map_err(|e| e.to_string())?;
    let mut cipher = vec![0; 65535];
    let n = noise
        .write_message(&plain, &mut cipher)
        .map_err(|e| e.to_string())?;
    packet(s, &cipher[..n])
}
fn run(
    mut stream: TcpStream,
    peer: String,
    is_host: bool,
    key: [u8; 32],
    events: &SyncSender<Event>,
    commands: &Receiver<Command>,
) {
    match establish(&mut stream, is_host, &key) {
        Ok((noise, capabilities)) => {
            run_established(stream, peer, is_host, noise, capabilities, events, commands)
        }
        Err(e) => {
            let _ = events.try_send(Event::Status(e));
        }
    }
}
fn establish(
    stream: &mut TcpStream,
    is_host: bool,
    key: &[u8; 32],
) -> Result<(snow::TransportState, u32), String> {
    let mut noise = handshake(stream, is_host, key)?;
    let capabilities = negotiate(stream, &mut noise)?;
    Ok((noise, capabilities))
}
fn run_established(
    mut stream: TcpStream,
    peer: String,
    is_host: bool,
    noise: snow::TransportState,
    capabilities: u32,
    events: &SyncSender<Event>,
    commands: &Receiver<Command>,
) {
    if let Err(e) = session(
        &mut stream,
        peer,
        is_host,
        noise,
        capabilities,
        events,
        commands,
    ) {
        let _ = events.try_send(Event::Status(e));
    }
}
fn session(
    stream: &mut TcpStream,
    peer: String,
    is_host: bool,
    mut noise: snow::TransportState,
    peer_capabilities: u32,
    events: &SyncSender<Event>,
    commands: &Receiver<Command>,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_millis(20)))
        .map_err(|e| e.to_string())?;
    if is_host {
        events
            .try_send(Event::Incoming { peer: peer.clone() })
            .map_err(|_| "UI unavailable")?;
    } else {
        let _ = events.try_send(Event::Status("Waiting for host approval".into()));
    }
    let mut approved = false;
    let mut permissions = Permissions::default();
    let mut buffered = Vec::new();
    let mut frame = Vec::new();
    let mut frame_size = 0;
    let mut temp = [0u8; 65536];
    let mut file: Option<(String, usize, Vec<u8>)> = None;
    let mut outgoing_frame: Option<(Vec<u8>, usize)> = None;
    let mut outgoing_file: Option<(Vec<u8>, usize)> = None;
    let mut file_turn = false;
    let mut builder: Option<Receiver<Result<crate::file_protocol::Sender, String>>> = None;
    let mut stream_sender: Option<crate::file_protocol::Sender> = None;
    let mut stream_receiver: Option<crate::file_protocol::Receiver> = None;
    let mut incoming_manifest: Option<(usize, Vec<u8>)> = None;
    let mut pending_manifest: Option<crate::file_protocol::Manifest> = None;
    let mut outgoing_manifest: Option<(Vec<u8>, usize)> = None;
    let mut resume_requests: std::collections::VecDeque<crate::file_protocol::ResumeRequest> =
        std::collections::VecDeque::new();
    let mut stream_active = false;
    let mut stream_progress = 0u64;
    let mut progress_at = std::time::Instant::now();
    let mut last_ping = std::time::Instant::now();
    let mut last_received = std::time::Instant::now();
    let mut auth_attempts = 0u8;
    let mut sent_bytes = 0u64;
    let mut received_bytes = 0u64;
    let mut metrics_at = std::time::Instant::now();
    loop {
        if last_ping.elapsed() >= Duration::from_secs(20) {
            send(stream, &mut noise, &Wire::Ping)?;
            last_ping = std::time::Instant::now();
        }
        if last_received.elapsed() >= Duration::from_secs(90) {
            return Err("Peer stopped responding".into());
        }
        if let Some(receiver) = &builder
            && let Ok(result) = receiver.try_recv()
        {
            builder = None;
            match result {
                Ok(sender) => {
                    let bytes = serde_json::to_vec(&sender.manifest).map_err(|e| e.to_string())?;
                    if bytes.len() > 262144 {
                        return Err("Manifest exceeds metadata limit".into());
                    }
                    send(stream, &mut noise, &Wire::TransferOfferStart(bytes.len()))?;
                    outgoing_manifest = Some((bytes, 0));
                    stream_sender = Some(sender);
                }
                Err(error) => {
                    let _ = events.try_send(Event::Status(error));
                }
            }
        }
        if metrics_at.elapsed() >= Duration::from_secs(1) {
            let _ = events.try_send(Event::Metrics {
                sent_bytes,
                received_bytes,
            });
            metrics_at = std::time::Instant::now();
        }
        // Drain several queued commands per turn so bursts of input or chat never
        // wait behind one 12 KiB frame chunk per loop iteration.
        for _ in 0..COMMANDS_PER_TURN {
            let command = match commands.try_recv() {
                Err(TryRecvError::Empty) => break,
                other => other,
            };
            match command {
                Ok(Command::SendPath(path))
                    if approved
                        && permissions.files
                        && builder.is_none()
                        && stream_sender.is_none() =>
                {
                    let (tx, rx) = mpsc::sync_channel(1);
                    builder = Some(rx);
                    thread::spawn(move || {
                        let _ = tx.send(crate::file_protocol::Sender::open(&path));
                    });
                }
                Ok(Command::SendPath(_)) if approved => {
                    let _ = events.try_send(Event::Status(
                        "Transfer busy or file permission disabled".into(),
                    ));
                }
                Ok(Command::ReceiveTransfer { directory })
                    if approved
                        && permissions.files
                        && pending_manifest.is_some()
                        && stream_receiver.is_none() =>
                {
                    let receiver = match crate::file_protocol::Receiver::open(
                        &directory,
                        pending_manifest.as_ref().unwrap().clone(),
                    ) {
                        Ok(receiver) => receiver,
                        Err(error) => {
                            let _ = events.try_send(Event::Status(error));
                            continue;
                        }
                    };
                    let requests = match receiver.requests() {
                        Ok(requests) => requests,
                        Err(error) => {
                            let _ = events.try_send(Event::Status(error));
                            continue;
                        }
                    };
                    pending_manifest = None;
                    send(stream, &mut noise, &Wire::TransferResume(requests))?;
                    events
                        .try_send(Event::TransferProgress {
                            id: receiver.manifest.id.clone(),
                            completed: receiver.progress(),
                            total: receiver.manifest.total,
                            receiving: true,
                        })
                        .map_err(|_| "UI unavailable")?;
                    if receiver.is_complete() {
                        send(
                            stream,
                            &mut noise,
                            &Wire::TransferFinished(receiver.manifest.id.clone()),
                        )?;
                        events
                            .try_send(Event::TransferComplete {
                                id: receiver.manifest.id.clone(),
                            })
                            .map_err(|_| "UI unavailable")?;
                    } else {
                        stream_receiver = Some(receiver);
                    }
                }
                Ok(Command::RejectTransfer) if approved => {
                    pending_manifest = None;
                    stream_receiver = None;
                    send(stream, &mut noise, &Wire::TransferRejected)?;
                }
                Ok(Command::Authenticate { password, totp })
                    if !is_host && !approved && auth_attempts < 3 =>
                {
                    validate_auth(&password, &totp)?;
                    auth_attempts += 1;
                    send(stream, &mut noise, &Wire::Authenticate { password, totp })?;
                }
                Ok(Command::SetPermissions(p)) if is_host && approved => {
                    permissions = p;
                    send(stream, &mut noise, &Wire::Permissions(p))?;
                    if !p.screen {
                        outgoing_frame = None;
                    }
                    if !p.files {
                        outgoing_file = None;
                        file = None;
                        builder = None;
                        stream_sender = None;
                        stream_receiver = None;
                        incoming_manifest = None;
                        pending_manifest = None;
                        outgoing_manifest = None;
                        resume_requests.clear();
                        stream_active = false;
                    }
                    if !p.input {
                        let _ = events.try_send(Event::Input(InputEvent::ReleaseAll));
                    }
                    events
                        .try_send(Event::Permissions(p))
                        .map_err(|_| "UI unavailable")?;
                }
                Ok(Command::Monitors(monitors))
                    if is_host && approved && permissions.screen && peer_capabilities & 8 != 0 =>
                {
                    validate_monitors(&monitors)?;
                    send(stream, &mut noise, &Wire::Monitors(monitors))?;
                }
                Ok(Command::SelectMonitor(id))
                    if !is_host
                        && approved
                        && permissions.screen
                        && id < 32
                        && peer_capabilities & 8 != 0 =>
                {
                    send(stream, &mut noise, &Wire::SelectMonitor(id))?;
                }
                Ok(Command::VideoFeedback { max_fps, quality })
                    if !is_host
                        && approved
                        && permissions.screen
                        && peer_capabilities & 16 != 0 =>
                {
                    validate_feedback(max_fps, quality)?;
                    send(
                        stream,
                        &mut noise,
                        &Wire::VideoFeedback { max_fps, quality },
                    )?;
                }
                Ok(Command::Audio {
                    sample_rate,
                    channels,
                    pcm,
                }) if is_host && approved && permissions.audio && peer_capabilities & 4 != 0 => {
                    validate_audio(sample_rate, channels, &pcm)?;
                    send(
                        stream,
                        &mut noise,
                        &Wire::Audio {
                            sample_rate,
                            channels,
                            pcm,
                        },
                    )?;
                }
                Ok(Command::Clipboard(text)) if approved && permissions.clipboard => {
                    if text.len() > 8192 {
                        return Err("Clipboard exceeds 8 KiB limit".into());
                    }
                    send(stream, &mut noise, &Wire::Clipboard(text))?;
                }
                Ok(Command::SendFile { name, bytes })
                    if approved && permissions.files && outgoing_file.is_none() =>
                {
                    if name.is_empty() || name.len() > 255 || bytes.len() > MAX_FILE {
                        return Err("Invalid file metadata or file exceeds 16 MiB".into());
                    }
                    send(
                        stream,
                        &mut noise,
                        &Wire::FileStart {
                            name,
                            size: bytes.len(),
                        },
                    )?;
                    if !bytes.is_empty() {
                        outgoing_file = Some((bytes, 0));
                    }
                }
                Ok(Command::Chat(text)) if approved => {
                    if text.len() > 4096 {
                        return Err("Chat message too long".into());
                    }
                    send(stream, &mut noise, &Wire::Chat(text))?;
                }
                Ok(Command::Stop) | Err(TryRecvError::Disconnected) => {
                    let _ = send(stream, &mut noise, &Wire::Bye);
                    return Ok(());
                }
                Ok(Command::Reject) if is_host && !approved => {
                    send(stream, &mut noise, &Wire::Rejected)?;
                    return Ok(());
                }
                Ok(Command::Accept { control: allow }) if is_host && !approved => {
                    permissions = Permissions {
                        input: allow,
                        ..Permissions::default()
                    };
                    send(stream, &mut noise, &Wire::Accepted(permissions))?;
                    approved = true;
                    events
                        .try_send(Event::Connected {
                            peer: peer.clone(),
                            control: permissions.input,
                        })
                        .map_err(|_| "UI unavailable")?;
                }
                Ok(Command::AcceptPermissions(p)) if is_host && !approved => {
                    permissions = p;
                    send(stream, &mut noise, &Wire::Accepted(p))?;
                    approved = true;
                    events
                        .try_send(Event::Connected {
                            peer: peer.clone(),
                            control: p.input,
                        })
                        .map_err(|_| "UI unavailable")?;
                    events
                        .try_send(Event::Permissions(p))
                        .map_err(|_| "UI unavailable")?;
                }
                Ok(Command::SendFile { .. }) if approved => {
                    let _ = events.try_send(Event::Status(
                        "File transfer busy; retry after it completes".into(),
                    ));
                }
                Ok(Command::Frame(jpeg))
                    if is_host && approved && permissions.screen && outgoing_frame.is_none() =>
                {
                    if jpeg.len() > MAX_FRAME {
                        return Err("Frame exceeds limit".into());
                    }
                    send(stream, &mut noise, &Wire::FrameStart(jpeg.len()))?;
                    outgoing_frame = Some((jpeg, 0));
                }
                Ok(Command::Input(input)) if !is_host && approved && permissions.input => {
                    validate_input(&input)?;
                    send(stream, &mut noise, &Wire::Input(input))?;
                }
                _ => {}
            }
        }
        let stream_ready =
            outgoing_manifest.is_some() || stream_active || !resume_requests.is_empty();
        let mut streamed_sent = false;
        if stream_ready && (file_turn || outgoing_frame.is_none()) {
            streamed_sent = true;
            if let Some((bytes, offset)) = &mut outgoing_manifest {
                let end = (*offset + CHUNK).min(bytes.len());
                send(
                    stream,
                    &mut noise,
                    &Wire::TransferManifestChunk(bytes[*offset..end].to_vec()),
                )?;
                *offset = end;
                if end == bytes.len() {
                    outgoing_manifest = None;
                }
            } else if let Some(sender) = &mut stream_sender {
                if !stream_active && let Some(request) = resume_requests.pop_front() {
                    sender.resume(&request)?;
                    stream_active = true;
                }
                if let Some((index, offset, bytes, finished)) = sender.next_chunk(CHUNK)? {
                    sent_bytes += bytes.len() as u64;
                    stream_progress += bytes.len() as u64;
                    send(
                        stream,
                        &mut noise,
                        &Wire::TransferChunk {
                            index,
                            offset,
                            bytes,
                            finished,
                        },
                    )?;
                    if finished {
                        stream_active = false;
                    }
                    if finished || progress_at.elapsed() >= Duration::from_millis(250) {
                        let _ = events.try_send(Event::TransferProgress {
                            id: sender.manifest.id.clone(),
                            completed: stream_progress,
                            total: sender.manifest.total,
                            receiving: false,
                        });
                        progress_at = std::time::Instant::now();
                    }
                }
            }
        }
        let sending_file = outgoing_file.is_some() && (file_turn || outgoing_frame.is_none());
        file_turn = !file_turn;
        let outgoing = if sending_file {
            &mut outgoing_file
        } else {
            &mut outgoing_frame
        };
        if let Some((bytes, offset)) = outgoing.as_mut().filter(|_| !streamed_sent) {
            let end = (*offset + CHUNK).min(bytes.len());
            let chunk = bytes[*offset..end].to_vec();
            sent_bytes += chunk.len() as u64;
            send(
                stream,
                &mut noise,
                &if sending_file {
                    Wire::FileChunk(chunk)
                } else {
                    Wire::Chunk(chunk)
                },
            )?;
            *offset = end;
            if end == bytes.len() {
                *outgoing = None;
            }
        }
        let active = outgoing_file.is_some() || outgoing_frame.is_some() || stream_ready;
        if active {
            stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        }
        let read = stream.read(&mut temp);
        if active {
            stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        }
        match read {
            Ok(0) => return Ok(()),
            Ok(n) => {
                last_received = std::time::Instant::now();
                received_bytes += n as u64;
                buffered.extend_from_slice(&temp[..n]);
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
        }
        while buffered.len() >= 4 {
            let n = u32::from_be_bytes(buffered[..4].try_into().unwrap()) as usize;
            if !(16..=65535).contains(&n) {
                return Err("Invalid packet length".into());
            }
            if buffered.len() < n + 4 {
                break;
            }
            let mut plain = vec![0; 65535];
            let count = noise
                .read_message(&buffered[4..n + 4], &mut plain)
                .map_err(|_| "Invalid encrypted record")?;
            buffered.drain(..n + 4);
            let w: Wire = serde_json::from_slice(&plain[..count]).map_err(|e| e.to_string())?;
            match w {
                Wire::Ping => {}
                Wire::TransferOfferStart(size)
                    if approved
                        && permissions.files
                        && incoming_manifest.is_none()
                        && pending_manifest.is_none()
                        && stream_receiver.is_none()
                        && size > 0
                        && size <= 262144 =>
                {
                    incoming_manifest = Some((size, Vec::new()));
                }
                Wire::TransferManifestChunk(chunk)
                    if approved && permissions.files && incoming_manifest.is_some() =>
                {
                    let (size, bytes) = incoming_manifest.as_mut().unwrap();
                    if chunk.is_empty() || chunk.len() > CHUNK || bytes.len() + chunk.len() > *size
                    {
                        return Err("Invalid manifest chunk".into());
                    }
                    bytes.extend(chunk);
                    if bytes.len() == *size {
                        let (_, bytes) = incoming_manifest.take().unwrap();
                        let manifest: crate::file_protocol::Manifest =
                            serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                        manifest.validate()?;
                        events
                            .try_send(Event::TransferOffer(manifest.clone()))
                            .map_err(|_| "UI unavailable")?;
                        pending_manifest = Some(manifest);
                    }
                }
                Wire::TransferResume(requests)
                    if approved
                        && permissions.files
                        && stream_sender.is_some()
                        && !stream_active
                        && resume_requests.is_empty() =>
                {
                    let sender = stream_sender.as_ref().unwrap();
                    if requests.len() > crate::file_protocol::MAX_ENTRIES {
                        return Err("Too many resume requests".into());
                    }
                    let mut seen = std::collections::HashSet::new();
                    let mut remaining = 0u64;
                    for request in &requests {
                        let entry = sender
                            .manifest
                            .entries
                            .get(request.index)
                            .ok_or("Invalid resume index")?;
                        if !seen.insert(request.index)
                            || request.offset > entry.size
                            || request.prefix_hash.len() != 64
                        {
                            return Err("Invalid resume request".into());
                        }
                        remaining += entry.size - request.offset;
                    }
                    stream_progress = sender.manifest.total - remaining;
                    resume_requests = requests.into();
                }
                Wire::TransferChunk {
                    index,
                    offset,
                    bytes,
                    finished,
                } if approved
                    && permissions.files
                    && stream_receiver.is_some()
                    && bytes.len() <= CHUNK =>
                {
                    let receiver = stream_receiver.as_mut().unwrap();
                    receiver.write_chunk(index, offset, &bytes, finished)?;
                    if finished || progress_at.elapsed() >= Duration::from_millis(250) {
                        let _ = events.try_send(Event::TransferProgress {
                            id: receiver.manifest.id.clone(),
                            completed: receiver.progress(),
                            total: receiver.manifest.total,
                            receiving: true,
                        });
                        progress_at = std::time::Instant::now();
                    }
                    if receiver.is_complete() {
                        let id = receiver.manifest.id.clone();
                        send(stream, &mut noise, &Wire::TransferFinished(id.clone()))?;
                        events
                            .try_send(Event::TransferComplete { id })
                            .map_err(|_| "UI unavailable")?;
                        stream_receiver = None;
                    }
                }
                Wire::TransferFinished(id)
                    if approved
                        && permissions.files
                        && stream_sender.as_ref().is_some_and(|s| s.manifest.id == id)
                        && !stream_active
                        && resume_requests.is_empty()
                        && outgoing_manifest.is_none() =>
                {
                    events
                        .try_send(Event::TransferComplete { id })
                        .map_err(|_| "UI unavailable")?;
                    stream_sender = None;
                }
                Wire::TransferRejected if approved => {
                    stream_sender = None;
                    outgoing_manifest = None;
                    resume_requests.clear();
                    stream_active = false;
                    let _ = events.try_send(Event::Status("Transfer rejected by receiver".into()));
                }
                Wire::Authenticate { password, totp }
                    if is_host && !approved && auth_attempts < 3 =>
                {
                    validate_auth(&password, &totp)?;
                    auth_attempts += 1;
                    events
                        .try_send(Event::Authentication { password, totp })
                        .map_err(|_| "Authentication queue full")?;
                }
                Wire::Permissions(p) if !is_host && approved => {
                    permissions = p;
                    if !p.screen {
                        frame.clear();
                        frame_size = 0;
                    }
                    if !p.files {
                        file = None;
                        outgoing_file = None;
                        builder = None;
                        stream_sender = None;
                        stream_receiver = None;
                        incoming_manifest = None;
                        pending_manifest = None;
                        outgoing_manifest = None;
                        resume_requests.clear();
                        stream_active = false;
                    }
                    events
                        .try_send(Event::Permissions(p))
                        .map_err(|_| "UI unavailable")?;
                }
                Wire::Monitors(monitors)
                    if !is_host && approved && permissions.screen && peer_capabilities & 8 != 0 =>
                {
                    validate_monitors(&monitors)?;
                    events
                        .try_send(Event::Monitors(monitors))
                        .map_err(|_| "UI unavailable")?;
                }
                Wire::SelectMonitor(id)
                    if is_host
                        && approved
                        && permissions.screen
                        && id < 32
                        && peer_capabilities & 8 != 0 =>
                {
                    events
                        .try_send(Event::SelectMonitor(id))
                        .map_err(|_| "UI unavailable")?;
                }
                Wire::VideoFeedback { max_fps, quality }
                    if is_host && approved && permissions.screen =>
                {
                    validate_feedback(max_fps, quality)?;
                    events
                        .try_send(Event::VideoFeedback { max_fps, quality })
                        .map_err(|_| "UI unavailable")?;
                }
                Wire::Audio {
                    sample_rate,
                    channels,
                    pcm,
                } if !is_host && approved && permissions.audio => {
                    validate_audio(sample_rate, channels, &pcm)?;
                    let _ = events.try_send(Event::Audio {
                        sample_rate,
                        channels,
                        pcm,
                    });
                }
                Wire::Clipboard(text)
                    if approved && permissions.clipboard && text.len() <= 8192 =>
                {
                    events
                        .try_send(Event::Clipboard(text))
                        .map_err(|_| "Clipboard queue full")?;
                }
                Wire::FileStart { name, size }
                    if approved
                        && permissions.files
                        && file.is_none()
                        && !name.is_empty()
                        && name.len() <= 255
                        && size <= MAX_FILE =>
                {
                    if size == 0 {
                        events
                            .try_send(Event::IncomingFile {
                                name,
                                bytes: Vec::new(),
                            })
                            .map_err(|_| "File queue full")?;
                    } else {
                        file = Some((name, size, Vec::new()));
                    }
                }
                Wire::FileChunk(chunk) if approved && permissions.files && file.is_some() => {
                    let (_, size, bytes) = file.as_mut().unwrap();
                    if chunk.is_empty() || chunk.len() > CHUNK || bytes.len() + chunk.len() > *size
                    {
                        return Err("Invalid file chunk".into());
                    }
                    bytes.extend(chunk);
                    if bytes.len() == *size {
                        let (name, _, bytes) = file.take().unwrap();
                        events
                            .try_send(Event::IncomingFile { name, bytes })
                            .map_err(|_| "File queue full")?;
                    }
                }
                Wire::Chat(text) if approved && text.len() <= 4096 => {
                    events
                        .try_send(Event::Chat(text))
                        .map_err(|_| "Chat queue full; session closed")?;
                }
                Wire::Bye | Wire::Rejected => return Ok(()),
                Wire::Accepted(p) if !is_host && !approved => {
                    approved = true;
                    permissions = p;
                    events
                        .try_send(Event::Connected {
                            peer: peer.clone(),
                            control: p.input,
                        })
                        .map_err(|_| "UI unavailable")?;
                    events
                        .try_send(Event::Permissions(p))
                        .map_err(|_| "UI unavailable")?;
                }
                Wire::FrameStart(size)
                    if !is_host
                        && approved
                        && permissions.screen
                        && size > 0
                        && size <= MAX_FRAME =>
                {
                    frame.clear();
                    frame_size = size;
                }
                Wire::Chunk(chunk)
                    if !is_host && approved && permissions.screen && frame_size > 0 =>
                {
                    if chunk.len() > CHUNK || frame.len() + chunk.len() > frame_size {
                        return Err("Invalid frame chunk".into());
                    }
                    frame.extend(chunk);
                    if frame.len() == frame_size {
                        let _ = events.try_send(Event::Frame {
                            jpeg: std::mem::take(&mut frame),
                        });
                        frame_size = 0;
                    }
                }
                Wire::Input(input) if is_host && approved && permissions.input => {
                    validate_input(&input)?;
                    // A momentarily full queue drops this event instead of ending
                    // the session; the viewer releases held keys on focus loss.
                    if let Err(mpsc::TrySendError::Disconnected(_)) =
                        events.try_send(Event::Input(input))
                    {
                        return Err("UI unavailable".into());
                    }
                }
                Wire::Clipboard(_) if approved && !permissions.clipboard => {}
                Wire::Input(_) if approved && !permissions.input => {}
                Wire::FileStart { .. } | Wire::FileChunk(_) if approved && !permissions.files => {}
                Wire::TransferOfferStart(_)
                | Wire::TransferManifestChunk(_)
                | Wire::TransferResume(_)
                | Wire::TransferChunk { .. }
                | Wire::TransferFinished(_)
                    if approved && !permissions.files => {}
                Wire::FrameStart(_)
                | Wire::Chunk(_)
                | Wire::Monitors(_)
                | Wire::SelectMonitor(_)
                | Wire::VideoFeedback { .. }
                    if approved && !permissions.screen => {}
                Wire::Audio { .. } if approved && !permissions.audio => {}
                _ => return Err("Message is not permitted in current session state".into()),
            }
        }
    }
}
pub(crate) fn validate_input(input: &InputEvent) -> Result<(), String> {
    match input {
        InputEvent::Scroll { x, y } if !(-100..=100).contains(x) || !(-100..=100).contains(y) => {
            Err("Scroll delta exceeds limit".into())
        }
        InputEvent::Move { x, y }
            if !x.is_finite()
                || !y.is_finite()
                || !(0.0..=1.0).contains(x)
                || !(0.0..=1.0).contains(y) =>
        {
            Err("Invalid coordinates".into())
        }
        InputEvent::Button { button, .. } if *button > 2 => Err("Invalid button".into()),
        InputEvent::Key { key, .. } if key.len() > 64 => Err("Invalid key".into()),
        InputEvent::Text(t) if t.len() > 4096 => Err("Text too long".into()),
        _ => Ok(()),
    }
}
impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Status(message) => f.debug_tuple("Status").field(message).finish(),
            Self::Authentication { .. } => f.write_str("Authentication { [redacted] }"),
            _ => f.write_str("Opendesk event"),
        }
    }
}
fn validate_auth(password: &str, totp: &str) -> Result<(), String> {
    if password.is_empty()
        || password.len() > 1024
        || totp.len() != 6
        || !totp.bytes().all(|b| b.is_ascii_digit())
    {
        Err("Malformed authentication request".into())
    } else {
        Ok(())
    }
}
fn negotiate(stream: &mut TcpStream, noise: &mut snow::TransportState) -> Result<u32, String> {
    // Bits: live permissions, streaming files, PCM audio, monitor selection, video feedback.
    send(
        stream,
        noise,
        &Wire::Hello {
            version: 2,
            capabilities: 31,
        },
    )?;
    let packet = read_packet(stream)?;
    let mut plain = vec![0; 65535];
    let n = noise
        .read_message(&packet, &mut plain)
        .map_err(|_| "Invalid protocol negotiation")?;
    match serde_json::from_slice::<Wire>(&plain[..n]).map_err(|_| "Invalid protocol negotiation")? {
        Wire::Hello {
            version: 2,
            capabilities,
        } if capabilities & 3 == 3 => Ok(capabilities & 31),
        _ => Err("Incompatible Opendesk protocol; version 2 required".into()),
    }
}
fn validate_monitors(monitors: &[MonitorInfo]) -> Result<(), String> {
    if monitors.len() > 32
        || monitors.iter().any(|m| {
            m.id >= 32
                || m.name.len() > 128
                || m.width == 0
                || m.height == 0
                || m.width > 16384
                || m.height > 16384
        })
    {
        Err("Invalid monitor list".into())
    } else {
        Ok(())
    }
}
fn validate_feedback(fps: u8, quality: u8) -> Result<(), String> {
    if !(1..=60).contains(&fps) || !(20..=95).contains(&quality) {
        Err("Invalid video feedback".into())
    } else {
        Ok(())
    }
}
fn validate_audio(rate: u32, channels: u8, pcm: &[i16]) -> Result<(), String> {
    if !(8000..=48000).contains(&rate)
        || !(1..=2).contains(&channels)
        || pcm.is_empty()
        || pcm.len() > 4096
        || !pcm.len().is_multiple_of(channels as usize)
    {
        Err("Invalid audio packet".into())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pair(key: [u8; 32], other: [u8; 32]) -> (Connection, Connection) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (ht, hr) = mpsc::sync_channel(8);
        let (hc, hcr) = mpsc::sync_channel(8);
        thread::spawn(move || {
            let (s, p) = listener.accept().unwrap();
            run(s, p.to_string(), true, key, &ht, &hcr);
            let _ = ht.try_send(Event::Disconnected);
        });
        let stream = TcpStream::connect(addr).unwrap();
        let (ct, cr) = mpsc::sync_channel(8);
        let (cc, ccr) = mpsc::sync_channel(8);
        thread::spawn(move || {
            run(stream, addr.to_string(), false, other, &ct, &ccr);
            let _ = ct.try_send(Event::Disconnected);
        });
        (
            Connection {
                events: hr,
                commands: hc,
                tunnel: None,
            },
            Connection {
                events: cr,
                commands: cc,
                tunnel: None,
            },
        )
    }
    fn next(c: &Connection) -> Event {
        loop {
            let event = c.events.recv_timeout(Duration::from_secs(5)).unwrap();
            if !matches!(event, Event::Permissions(_) | Event::Metrics { .. }) {
                return event;
            }
        }
    }
    #[test]
    fn default_port_is_added_only_when_missing() {
        assert_eq!(with_default_port(" 192.168.1.20 "), "192.168.1.20:7878");
        assert_eq!(with_default_port("192.168.1.20:9000"), "192.168.1.20:9000");
        assert_eq!(with_default_port("lab-pc"), "lab-pc:7878");
        assert_eq!(with_default_port("lab-pc:9000"), "lab-pc:9000");
        assert_eq!(with_default_port("fe80::1"), "[fe80::1]:7878");
        assert_eq!(with_default_port("[fe80::1]:9000"), "[fe80::1]:9000");
    }
    #[test]
    fn secret_validation() {
        assert!(secret(&generate_secret()).is_ok());
        assert!(secret("password").is_err());
        assert!(secret(&"z".repeat(64)).is_err());
        let code = generate_code();
        assert_eq!(code.len(), 23);
        assert_eq!(
            secret(&code).unwrap(),
            secret(&code.to_lowercase()).unwrap()
        );
        assert!(secret("AAAAA-AAAAA-AAAAA-AAAAI").is_err());
        assert!(secret("123456").is_err());
    }
    #[test]
    fn incompatible_protocol_is_rejected_before_consent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let (_cmd, commands) = mpsc::sync_channel(8);
        thread::spawn(move || {
            let (stream, peer) = listener.accept().unwrap();
            run(stream, peer.to_string(), true, [72; 32], &tx, &commands);
            let _ = tx.try_send(Event::Disconnected);
        });
        let mut stream = TcpStream::connect(address).unwrap();
        let mut noise = handshake(&mut stream, false, &[72; 32]).unwrap();
        send(
            &mut stream,
            &mut noise,
            &Wire::Hello {
                version: 1,
                capabilities: 31,
            },
        )
        .unwrap();
        assert!(
            matches!(rx.recv_timeout(Duration::from_secs(5)).unwrap(),Event::Status(s)if s.contains("Incompatible"))
        );
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            Event::Disconnected
        ));
    }
    #[test]
    fn encrypted_auth_and_live_permission_revocation() {
        let (h, c) = pair([44; 32], [44; 32]);
        let _ = next(&h);
        let _ = next(&c);
        c.commands
            .send(Command::Authenticate {
                password: "private".into(),
                totp: "123456".into(),
            })
            .unwrap();
        let auth = next(&h);
        assert!(!format!("{auth:?}").contains("private"));
        assert!(matches!(auth,Event::Authentication{password,..}if password=="private"));
        h.commands
            .send(Command::AcceptPermissions(Permissions {
                audio: true,
                input: true,
                ..Permissions::default()
            }))
            .unwrap();
        let _ = next(&h);
        let _ = next(&c);
        h.commands
            .send(Command::Audio {
                sample_rate: 48000,
                channels: 2,
                pcm: vec![1; 100],
            })
            .unwrap();
        assert!(matches!(next(&c),Event::Audio{pcm,..}if pcm.len()==100));
        c.commands.send(Command::SelectMonitor(1)).unwrap();
        assert!(matches!(next(&h), Event::SelectMonitor(1)));
        c.commands
            .send(Command::VideoFeedback {
                max_fps: 20,
                quality: 70,
            })
            .unwrap();
        assert!(matches!(
            next(&h),
            Event::VideoFeedback {
                max_fps: 20,
                quality: 70
            }
        ));
        h.commands
            .send(Command::SetPermissions(Permissions {
                clipboard: false,
                input: false,
                audio: false,
                ..Permissions::default()
            }))
            .unwrap();
        assert!(matches!(next(&h), Event::Input(InputEvent::ReleaseAll)));
        loop {
            if matches!(c.events.recv_timeout(Duration::from_secs(2)).unwrap(),Event::Permissions(p)if !p.clipboard)
            {
                break;
            }
        }
        c.commands
            .send(Command::Clipboard("blocked".into()))
            .unwrap();
        c.commands
            .send(Command::Input(InputEvent::Text("blocked".into())))
            .unwrap();
        assert!(
            h.events
                .recv_timeout(Duration::from_millis(100))
                .map(|e| matches!(e, Event::Permissions(_) | Event::Metrics { .. }))
                .unwrap_or(true)
        );
        c.commands
            .send(Command::Chat("still connected".into()))
            .unwrap();
        assert!(matches!(next(&h), Event::Chat(_)));
        c.commands.send(Command::Stop).unwrap();
    }
    #[test]
    fn streaming_folder_resumes_verified_partial_over_network() {
        use crate::file_protocol::{Receiver as FileReceiver, Sender as FileSender};
        let temp =
            std::env::temp_dir().join(format!("opendesk-network-files-{}", rand::random::<u64>()));
        let folder = temp.join("source");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("large.bin"), vec![7; 250_000]).unwrap();
        std::fs::write(folder.join("empty"), []).unwrap();
        let output = temp.join("output");
        let mut source = FileSender::open(&folder).unwrap();
        let mut receiver = FileReceiver::open(&output, source.manifest.clone()).unwrap();
        let request = receiver
            .requests()
            .unwrap()
            .into_iter()
            .find(|r| source.manifest.entries[r.index].size > 0)
            .unwrap();
        source.resume(&request).unwrap();
        let (i, o, b, f) = source.next_chunk(1024).unwrap().unwrap();
        receiver.write_chunk(i, o, &b, f).unwrap();
        drop(receiver);
        let (h, c) = pair([45; 32], [45; 32]);
        let _ = next(&h);
        let _ = next(&c);
        h.commands.send(Command::Accept { control: false }).unwrap();
        let _ = next(&h);
        let _ = next(&c);
        h.commands.send(Command::SendPath(folder)).unwrap();
        assert!(matches!(next(&c), Event::TransferOffer(_)));
        c.commands
            .send(Command::ReceiveTransfer {
                directory: output.clone(),
            })
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let mut resumed = false;
        loop {
            match c
                .events
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .unwrap()
            {
                Event::TransferProgress { completed, .. } => resumed |= completed >= 1024,
                Event::TransferComplete { .. } => break,
                Event::Status(s) => panic!("{s}"),
                _ => {}
            }
        }
        assert!(resumed);
        assert_eq!(
            std::fs::read(output.join("source/large.bin")).unwrap(),
            vec![7; 250_000]
        );
        assert!(output.join("source/empty").exists());
        c.commands.send(Command::Stop).unwrap();
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn simultaneous_large_files_and_screen_do_not_deadlock() {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let transfer_next = |connection: &Connection| {
            loop {
                let event = connection
                    .events
                    .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                    .unwrap();
                if !matches!(event, Event::Permissions(_) | Event::Metrics { .. }) {
                    break event;
                }
            }
        };
        let (h, c) = pair([12; 32], [12; 32]);
        let _ = next(&h);
        let _ = next(&c);
        h.commands.send(Command::Accept { control: false }).unwrap();
        let _ = next(&h);
        let _ = next(&c);
        let data = vec![255; 2 * 1024 * 1024];
        h.commands.send(Command::Frame(data.clone())).unwrap();
        h.commands
            .send(Command::SendFile {
                name: "from-host".into(),
                bytes: data.clone(),
            })
            .unwrap();
        c.commands
            .send(Command::SendFile {
                name: "from-viewer".into(),
                bytes: data.clone(),
            })
            .unwrap();
        assert!(matches!(transfer_next(&h),Event::IncomingFile{bytes,..}if bytes==data));
        let mut got_frame = false;
        let mut got_file = false;
        for _ in 0..2 {
            match transfer_next(&c) {
                Event::Frame { jpeg } => {
                    assert_eq!(jpeg, data);
                    got_frame = true
                }
                Event::IncomingFile { bytes, .. } => {
                    assert_eq!(bytes, data);
                    got_file = true
                }
                e => panic!("{e:?}"),
            }
        }
        assert!(got_frame && got_file);
        c.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Disconnected));
    }
    #[test]
    fn actual_nonblocking_listener_accepts_session() {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap().to_string();
        let key = generate_secret();
        let h = host_listener(reservation, secret(&key).unwrap()).unwrap();
        let _ = next(&h);
        let c = connect(&address, &key).unwrap();
        assert!(matches!(next(&h), Event::Incoming { .. }));
        let _ = next(&c);
        h.commands.send(Command::Accept { control: false }).unwrap();
        assert!(matches!(next(&h), Event::Connected { .. }));
        assert!(matches!(next(&c), Event::Connected { .. }));
        c.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Disconnected));
    }
    #[test]
    fn listener_survives_unauthenticated_and_silent_peers() {
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap().to_string();
        let key = generate_secret();
        let h = host_listener(reservation, secret(&key).unwrap()).unwrap();
        let _ = next(&h);
        let wrong = connect(&address, &generate_secret()).unwrap();
        assert!(matches!(next(&h), Event::Status(s) if s.contains("Ignored connection")));
        let _ = wrong.commands.send(Command::Stop);
        let silent = TcpStream::connect(&address).unwrap();
        assert!(matches!(next(&h), Event::Status(s) if s.contains("Ignored connection")));
        drop(silent);
        let c = connect(&address, &key).unwrap();
        assert!(matches!(next(&h), Event::Incoming { .. }));
        let _ = next(&c);
        h.commands.send(Command::Accept { control: false }).unwrap();
        assert!(matches!(next(&h), Event::Connected { .. }));
        assert!(matches!(next(&c), Event::Connected { .. }));
        c.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Disconnected));
    }
    #[test]
    fn relay_preserves_encryption_and_consent() {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap().to_string();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let server = thread::spawn(move || crate::relay::serve(listener, flag).unwrap());
        let key = generate_secret();
        let h = host_relay(&address, &key).unwrap();
        assert!(matches!(next(&h),Event::Status(s)if s.contains("Waiting at relay")));
        let c = connect_relay(&address, &key).unwrap();
        let event = next(&h);
        assert!(matches!(event, Event::Incoming { .. }), "{event:?}");
        let _ = next(&c);
        h.commands.send(Command::Accept { control: false }).unwrap();
        let _ = next(&h);
        let _ = next(&c);
        c.commands
            .send(Command::Clipboard("clipboard".into()))
            .unwrap();
        assert!(matches!(next(&h),Event::Clipboard(s)if s=="clipboard"));
        h.commands.send(Command::Frame(vec![4; 100_000])).unwrap();
        assert!(matches!(next(&c),Event::Frame{jpeg}if jpeg==vec![4;100_000]));
        c.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Disconnected));
        stop.store(true, Ordering::Relaxed);
        server.join().unwrap();
    }
    #[test]
    fn malicious_peer_cannot_bypass_consent() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::sync_channel(8);
        let (_commands, cr) = mpsc::sync_channel(8);
        thread::spawn(move || {
            let (s, p) = listener.accept().unwrap();
            run(s, p.to_string(), true, [11; 32], &tx, &cr);
            let _ = tx.try_send(Event::Disconnected);
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        let mut noise = handshake(&mut stream, false, &[11; 32]).unwrap();
        negotiate(&mut stream, &mut noise).unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Incoming { .. }
        ));
        send(
            &mut stream,
            &mut noise,
            &Wire::Input(InputEvent::Text("unauthorized".into())),
        )
        .unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Status(_)
        ));
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(2)).unwrap(),
            Event::Disconnected
        ));
    }
    #[test]
    fn chat_and_files_require_approval_and_round_trip() {
        let (h, c) = pair([9; 32], [9; 32]);
        let _ = next(&h);
        let _ = next(&c);
        c.commands.send(Command::Chat("premature".into())).unwrap();
        c.commands
            .send(Command::SendFile {
                name: "early".into(),
                bytes: vec![1; 100],
            })
            .unwrap();
        assert!(h.events.recv_timeout(Duration::from_millis(100)).is_err());
        h.commands.send(Command::Accept { control: false }).unwrap();
        let _ = next(&h);
        let _ = next(&c);
        c.commands.send(Command::Chat("hello".into())).unwrap();
        assert!(matches!(next(&h),Event::Chat(t)if t=="hello"));
        let bytes = vec![255; 150_000];
        c.commands
            .send(Command::SendFile {
                name: "example.bin".into(),
                bytes: bytes.clone(),
            })
            .unwrap();
        match next(&h) {
            Event::IncomingFile { name, bytes: b } => {
                assert_eq!(name, "example.bin");
                assert_eq!(b, bytes)
            }
            e => panic!("{e:?}"),
        };
        h.commands
            .send(Command::SendFile {
                name: "empty".into(),
                bytes: Vec::new(),
            })
            .unwrap();
        assert!(matches!(next(&c),Event::IncomingFile{bytes,..}if bytes.is_empty()));
        h.commands.send(Command::Stop).unwrap();
    }
    #[test]
    fn approved_frame_and_control() {
        let (h, c) = pair([7; 32], [7; 32]);
        assert!(matches!(next(&h), Event::Incoming { .. }));
        assert!(matches!(next(&c), Event::Status(_)));
        h.commands.send(Command::Frame(vec![1; 99])).unwrap();
        assert!(c.events.recv_timeout(Duration::from_millis(100)).is_err());
        h.commands.send(Command::Accept { control: true }).unwrap();
        assert!(matches!(next(&h), Event::Connected { control: true, .. }));
        assert!(matches!(next(&c), Event::Connected { control: true, .. }));
        let bytes = vec![42; 150_000];
        h.commands.send(Command::Frame(bytes.clone())).unwrap();
        match next(&c) {
            Event::Frame { jpeg } => assert_eq!(jpeg, bytes),
            e => panic!("{e:?}"),
        };
        c.commands
            .send(Command::Input(InputEvent::Text("hello".into())))
            .unwrap();
        assert!(matches!(next(&h), Event::Input(InputEvent::Text(_))));
        c.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Disconnected));
    }
    #[test]
    fn wrong_secret_is_rejected() {
        let (h, c) = pair([7; 32], [8; 32]);
        assert!(matches!(next(&h), Event::Status(_)));
        assert!(matches!(next(&h), Event::Disconnected));
        assert!(matches!(next(&c), Event::Status(_)));
        assert!(matches!(next(&c), Event::Disconnected));
    }
    #[test]
    fn view_only_blocks_input_and_reject_closes() {
        let (h, c) = pair([2; 32], [2; 32]);
        assert!(matches!(next(&h), Event::Incoming { .. }));
        let _ = next(&c);
        h.commands.send(Command::Accept { control: false }).unwrap();
        assert!(matches!(next(&h), Event::Connected { control: false, .. }));
        assert!(matches!(next(&c), Event::Connected { control: false, .. }));
        c.commands
            .send(Command::Input(InputEvent::Text("blocked".into())))
            .unwrap();
        assert!(h.events.recv_timeout(Duration::from_millis(100)).is_err());
        c.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Disconnected));
        let (h, c) = pair([3; 32], [3; 32]);
        let _ = next(&h);
        let _ = next(&c);
        h.commands.send(Command::Reject).unwrap();
        assert!(matches!(next(&c), Event::Disconnected));
    }
    #[test]
    fn pending_host_can_stop() {
        let key = generate_secret();
        let h = host("127.0.0.1:0", &key).unwrap();
        h.commands.send(Command::Stop).unwrap();
        assert!(matches!(next(&h), Event::Status(_)));
        assert!(matches!(next(&h), Event::Disconnected));
    }
}
