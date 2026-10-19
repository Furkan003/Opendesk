//! Moves session work off the UI thread. Frames are decoded here and only the newest
//! one is kept, and host input is injected here, so a busy or minimized window neither
//! stalls remote control nor lets queues grow until the session closes.
use crate::platform::InputController;
use eframe::egui;
use opendesk::network::{Connection, Event};
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError},
    },
    time::Duration,
};

const UI_QUEUE: usize = 1024;

#[derive(Default)]
pub struct FrameSlot {
    /// Newest decoded frame with its JPEG bytes (kept for recording).
    pub latest: Option<(Vec<u8>, egui::ColorImage)>,
    /// Frames and bytes received since the UI last read the counters.
    pub frames: usize,
    pub bytes: usize,
}

pub struct Pump {
    input: Sender<Option<usize>>,
    pub frames: Arc<Mutex<FrameSlot>>,
}
impl Pump {
    /// Replaces the connection's event receiver with the pump's UI queue.
    pub fn attach(connection: &mut Connection, ctx: egui::Context) -> Self {
        let (out, ui) = mpsc::sync_channel(UI_QUEUE);
        let source = std::mem::replace(&mut connection.events, ui);
        let (input, control) = mpsc::channel();
        let frames = Arc::new(Mutex::new(FrameSlot::default()));
        let slot = frames.clone();
        std::thread::spawn(move || run(source, out, control, slot, ctx));
        Self { input, frames }
    }
    /// `Some(monitor)` injects remote input on that display; `None` stops injecting.
    pub fn set_input(&self, monitor: Option<usize>) {
        let _ = self.input.send(monitor);
    }
}

fn droppable(event: &Event) -> bool {
    matches!(
        event,
        Event::Audio { .. }
            | Event::Metrics { .. }
            | Event::Status(_)
            | Event::TransferProgress { .. }
    )
}

fn forward(out: &SyncSender<Event>, event: Event) -> bool {
    match out.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(event)) => droppable(&event) || out.send(event).is_ok(),
        Err(TrySendError::Disconnected(_)) => false,
    }
}

fn run(
    source: Receiver<Event>,
    out: SyncSender<Event>,
    control: Receiver<Option<usize>>,
    frames: Arc<Mutex<FrameSlot>>,
    ctx: egui::Context,
) {
    let mut input: Option<InputController> = None;
    loop {
        for monitor in control.try_iter() {
            input = None;
            if let Some(monitor) = monitor {
                match InputController::new(monitor) {
                    Ok(controller) => input = Some(controller),
                    Err(e) => {
                        if !forward(&out, Event::Status(format!("Control unavailable: {e}"))) {
                            return;
                        }
                    }
                }
            }
        }
        let event = match source.recv_timeout(Duration::from_millis(50)) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                let _ = out.send(Event::Disconnected);
                ctx.request_repaint();
                return;
            }
        };
        match event {
            Event::Input(event) => {
                if let Some(controller) = &mut input
                    && let Err(e) = controller.apply(event)
                    && !forward(&out, Event::Status(format!("Input: {e}")))
                {
                    return;
                }
            }
            Event::Frame { jpeg } => {
                let Ok(image) = crate::decode_frame(&jpeg) else {
                    continue;
                };
                let rgba = image.to_rgba8();
                let color = egui::ColorImage::from_rgba_unmultiplied(
                    [rgba.width() as usize, rgba.height() as usize],
                    &rgba,
                );
                let mut slot = frames.lock().unwrap_or_else(|e| e.into_inner());
                slot.frames += 1;
                slot.bytes += jpeg.len();
                slot.latest = Some((jpeg, color));
                drop(slot);
                ctx.request_repaint();
            }
            event => {
                let closing = matches!(event, Event::Disconnected);
                if !forward(&out, event) || closing {
                    ctx.request_repaint();
                    return;
                }
                ctx.request_repaint();
            }
        }
    }
}
