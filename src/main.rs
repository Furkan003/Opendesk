#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod audio;
mod class_ui;
mod design;
mod platform;
mod pump;
mod recording;
mod settings;
use eframe::egui;
use opendesk::network::{self, Command, Connection, Event, InputEvent, MonitorInfo, Permissions};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn argument_value(name: &str) -> Option<String> {
    let args: Vec<_> = std::env::args().collect();
    args.windows(2).find(|a| a[0] == name).map(|a| a[1].clone())
}
fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--smoke-audio") {
        match audio::AudioCapture::start()
            .and_then(|capture| audio::AudioPlayer::new().map(|player| (capture, player)))
        {
            Ok((_capture, _player)) => println!("System audio capture and playback initialized"),
            Err(e) => {
                eprintln!("Audio unavailable: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    let smoke = std::env::args().any(|a| a == "--smoke-test");
    let capture = argument_value("--ui-capture").is_some();
    if let Some(role) = argument_value("--set-role") {
        // Used by the Windows installer to record the role chosen during setup.
        let Some(role) = settings::Role::parse(&role) else {
            eprintln!("Unknown role: {role} (personal, teacher, student)");
            std::process::exit(2);
        };
        let mut stored = settings::Settings::load();
        stored.role = Some(role);
        if let Err(e) = stored.save() {
            eprintln!("Could not save role: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }
    if std::env::args().any(|a| a == "--version") {
        println!("Opendesk {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if std::env::args().any(|a| a == "--self-test") {
        let result = platform::capture_jpeg().and_then(|jpeg| {
            decode_frame(&jpeg).map(|img| (jpeg.len(), img.width(), img.height()))
        });
        match result {
            Ok((bytes, w, h)) => println!("Capture OK: {w}x{h}, {bytes} JPEG bytes"),
            Err(e) => {
                eprintln!("Capture failed: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    eframe::run_native(
        "Opendesk",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1080.0, 760.0])
                .with_min_inner_size([720.0, 500.0])
                .with_title("Opendesk")
                .with_visible(!smoke && !capture)
                .with_icon(design::icon_rgba()),
            ..Default::default()
        },
        Box::new(move |cc| {
            design::configure_fonts(&cc.egui_ctx);
            let mut desk = Desk::default();
            desk.egui_ctx = cc.egui_ctx.clone();
            Ok(Box::new(desk))
        }),
    )
}
enum UpdateResult {
    Checked(opendesk::update::Available),
    Downloaded(std::path::PathBuf),
}
/// About 30 seconds of consecutive capture failures before sharing stops.
const MAX_CAPTURE_FAILURES: u32 = 120;
type CaptureSettings = Arc<std::sync::Mutex<(usize, u8, u64, bool, u32)>>;
fn retry_delay(enabled: bool, attempt: u8) -> Option<Duration> {
    (enabled && attempt < 5).then(|| Duration::from_secs(1u64 << attempt))
}
fn sync_remote_modifiers(
    output: &mut Vec<InputEvent>,
    current: &mut egui::Modifiers,
    next: egui::Modifiers,
) {
    for (name, old, new) in [
        ("Control", current.ctrl, next.ctrl),
        ("Alt", current.alt, next.alt),
        ("Shift", current.shift, next.shift),
        ("Meta", current.mac_cmd, next.mac_cmd),
    ] {
        if old != new {
            output.push(InputEvent::Key {
                key: name.into(),
                pressed: new,
            });
        }
    }
    *current = next;
}
fn printable_remote_key(key: egui::Key) -> bool {
    let name = format!("{key:?}");
    name.len() == 1
        || name.starts_with("Num")
        || matches!(
            key,
            egui::Key::Space
                | egui::Key::Colon
                | egui::Key::Comma
                | egui::Key::Backslash
                | egui::Key::Slash
                | egui::Key::Minus
                | egui::Key::Equals
                | egui::Key::Semicolon
                | egui::Key::Period
                | egui::Key::Plus
                | egui::Key::OpenBracket
                | egui::Key::CloseBracket
                | egui::Key::Backtick
                | egui::Key::Quote
                | egui::Key::Pipe
                | egui::Key::Questionmark
                | egui::Key::Exclamationmark
        )
}
fn translate_remote_events(
    events: &[egui::Event],
    initial: egui::Modifiers,
    final_modifiers: egui::Modifiers,
    pointer_hovered: bool,
) -> Vec<InputEvent> {
    let mut output = Vec::new();
    let mut current = initial;
    let has_paste = events.iter().any(|e| matches!(e, egui::Event::Paste(_)));
    for event in events {
        match event {
            egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } => {
                // egui represents native paste as both Ctrl+V and Paste on some backends.
                // Send the supplied text once, without first pasting the remote clipboard.
                if has_paste && *key == egui::Key::V && (modifiers.ctrl || modifiers.command) {
                    continue;
                }
                sync_remote_modifiers(&mut output, &mut current, *modifiers);
                if !pressed
                    || !printable_remote_key(*key)
                    || modifiers.ctrl
                    || modifiers.alt
                    || modifiers.mac_cmd
                {
                    output.push(InputEvent::Key {
                        key: format!("{key:?}"),
                        pressed: *pressed,
                    });
                }
            }
            egui::Event::Text(text) if !current.ctrl && !current.alt && !current.mac_cmd => {
                output.push(InputEvent::Text(text.clone()))
            }
            egui::Event::Paste(text) => {
                sync_remote_modifiers(&mut output, &mut current, egui::Modifiers::NONE);
                let mut chunk = String::new();
                for c in text.chars() {
                    if chunk.len() + c.len_utf8() > 4096 {
                        output.push(InputEvent::Text(std::mem::take(&mut chunk)));
                    }
                    chunk.push(c);
                }
                if !chunk.is_empty() {
                    output.push(InputEvent::Text(chunk));
                }
            }
            egui::Event::PointerButton {
                button,
                pressed,
                modifiers,
                ..
            } if pointer_hovered || !*pressed => {
                sync_remote_modifiers(&mut output, &mut current, *modifiers);
                let button = match button {
                    egui::PointerButton::Primary => 0,
                    egui::PointerButton::Secondary => 1,
                    egui::PointerButton::Middle => 2,
                    _ => continue,
                };
                output.push(InputEvent::Button {
                    button,
                    pressed: *pressed,
                });
            }
            egui::Event::MouseWheel {
                delta, modifiers, ..
            } => {
                sync_remote_modifiers(&mut output, &mut current, *modifiers);
                output.push(InputEvent::Scroll {
                    x: (-delta.x / 20.).clamp(-100., 100.) as i32,
                    y: (-delta.y / 20.).clamp(-100., 100.) as i32,
                });
            }
            _ => {}
        }
    }
    sync_remote_modifiers(&mut output, &mut current, final_modifiers);
    output
}
struct Desk {
    transport_ready: bool,
    ui_capture: Option<std::path::PathBuf>,
    ui_capture_frames: u8,
    ui_page: design::Page,
    ui_tab: Option<design::Tab>,
    device_search: String,
    add_device: bool,
    connection_input: String,
    peer_label: String,
    update_work: Option<std::sync::mpsc::Receiver<Result<UpdateResult, String>>>,
    update_available: Option<opendesk::update::Available>,
    staged_update: Option<std::path::PathBuf>,
    unattended: Option<opendesk::unattended::Policy>,
    unattended_password: String,
    unattended_token: String,
    enrollment: Option<opendesk::unattended::Enrollment>,
    use_unattended: bool,
    transfer_offer: Option<opendesk::file_protocol::Manifest>,
    transfer_status: String,
    settings: settings::Settings,
    /// This computer's LAN address, shown to viewers on the same network.
    lan_ip: Option<std::net::IpAddr>,
    /// Audio playback could not start; do not retry for every packet.
    audio_failed: bool,
    class: class_ui::Classroom,
    computer_name: String,
    /// Scale the remote screen to the window (otherwise 1:1 with scrolling).
    fit_to_window: bool,
    /// A viewer's manual quality change still has to reach the host.
    pending_feedback: bool,
    device_name: String,
    invite: String,
    permissions: Permissions,
    monitors: Vec<MonitorInfo>,
    capture_settings: CaptureSettings,
    max_width: u32,
    actual_fps: f32,
    auto_reconnect: bool,
    retry_at: Option<std::time::Instant>,
    retries: u8,
    fullscreen: bool,
    adaptive: bool,
    feedback_at: std::time::Instant,
    frame_window: usize,
    bytes_window: usize,
    video_stats: String,
    recorder: Option<recording::Recorder>,
    record_path: String,
    frame_size: Option<(u32, u32)>,
    audio_capture: Option<audio::AudioCapture>,
    audio_player: Option<audio::AudioPlayer>,
    smoke: bool,
    smoke_frames: u8,
    relay: bool,
    relay_address: String,
    /// Code-only Internet sessions over iroh; no address or server needed.
    internet: bool,
    connecting_host: bool,
    pending_release: bool,
    capture_errors: Option<std::sync::mpsc::Receiver<String>>,
    connecting: Option<std::sync::mpsc::Receiver<Result<Connection, String>>>,
    received_clipboard: Option<String>,
    address: String,
    secret: String,
    bind: String,
    status: String,
    connection: Option<Connection>,
    hosting: bool,
    connected: bool,
    control: bool,

    incoming: Option<String>,
    texture: Option<egui::TextureHandle>,
    capture_stop: Arc<AtomicBool>,
    /// Decodes frames and injects host input off the UI thread.
    pump: Option<pump::Pump>,
    egui_ctx: egui::Context,
    modifiers: egui::Modifiers,
    monitor: usize,
    quality: u8,
    fps: u64,
    viewer_focused: bool,
    chat: String,
    messages: Vec<String>,
    outgoing: String,
    inbox: String,
    incoming_file: Option<(String, Vec<u8>)>,
}
impl Default for Desk {
    fn default() -> Self {
        let mut connection_settings = settings::Settings::load();
        if let Some(theme) = argument_value("--ui-theme") {
            connection_settings.dark = theme == "dark";
        }
        if connection_settings.role.is_none()
            && let Some(role) = settings::Settings::installed_role()
        {
            connection_settings.role = Some(role);
            let _ = connection_settings.save();
        }
        let prefs = connection_settings.preferences.clone().sanitized();
        Self {
            transport_ready: false,
            ui_capture: argument_value("--ui-capture").map(std::path::PathBuf::from),
            ui_capture_frames: 0,
            ui_page: match argument_value("--ui-page").as_deref() {
                Some("devices") => design::Page::Devices,
                Some("settings") => design::Page::Settings,
                Some("class") => design::Page::Classroom,
                Some(_) => design::Page::Home,
                None => match connection_settings.role {
                    Some(settings::Role::Teacher | settings::Role::Student) => {
                        design::Page::Classroom
                    }
                    _ => design::Page::Home,
                },
            },
            ui_tab: Some(design::Tab::Chat),
            device_search: String::new(),
            add_device: false,
            connection_input: String::new(),
            peer_label: String::new(),
            update_work: None,
            update_available: None,
            staged_update: None,
            unattended: opendesk::unattended::Policy::load().ok().flatten(),
            unattended_password: String::new(),
            unattended_token: String::new(),
            enrollment: None,
            use_unattended: false,
            transfer_offer: None,
            transfer_status: String::new(),
            lan_ip: network::local_ip(),
            device_name: String::new(),
            invite: String::new(),
            permissions: Permissions::default(),
            monitors: Vec::new(),
            capture_settings: Arc::new(std::sync::Mutex::new((
                0,
                prefs.quality,
                prefs.fps,
                true,
                prefs.max_width,
            ))),
            max_width: prefs.max_width,
            actual_fps: 0.0,
            auto_reconnect: prefs.auto_reconnect,
            retry_at: None,
            retries: 0,
            fullscreen: false,
            adaptive: prefs.adaptive,
            feedback_at: std::time::Instant::now(),
            frame_window: 0,
            bytes_window: 0,
            video_stats: String::new(),
            recorder: None,
            record_path: prefs.record_path.clone(),
            frame_size: None,
            audio_capture: None,
            audio_player: None,
            smoke: std::env::args().any(|a| a == "--smoke-test"),
            smoke_frames: 0,
            relay: connection_settings.relay,
            relay_address: connection_settings.relay_address.clone(),
            internet: !connection_settings.manual_route,
            connecting_host: false,
            pending_release: false,
            capture_errors: None,
            connecting: None,
            received_clipboard: None,
            address: String::new(),
            secret: opendesk::unattended::load_pairing_code()
                .unwrap_or_else(|_| network::generate_code()),
            bind: prefs.bind.clone(),
            status: "Ready. Both computers must run Opendesk.".into(),
            connection: None,
            hosting: false,
            connected: false,
            control: false,

            incoming: None,
            texture: None,
            capture_stop: Arc::new(AtomicBool::new(true)),
            pump: None,
            egui_ctx: egui::Context::default(),
            modifiers: egui::Modifiers::NONE,
            monitor: 0,
            quality: prefs.quality,
            fps: prefs.fps,
            viewer_focused: false,
            chat: String::new(),
            messages: Vec::new(),
            outgoing: String::new(),
            inbox: prefs.inbox.clone(),
            incoming_file: None,
            audio_failed: false,
            class: class_ui::Classroom::default(),
            computer_name: class_ui::computer_name(),
            fit_to_window: true,
            pending_feedback: false,
            settings: connection_settings,
        }
    }
}
impl Desk {
    fn send(&self, command: Command) -> bool {
        self.connection
            .as_ref()
            .is_some_and(|c| c.commands.try_send(command).is_ok())
    }
    fn attach(&mut self, mut connection: Connection) {
        self.pump = Some(pump::Pump::attach(&mut connection, self.egui_ctx.clone()));
        self.connection = Some(connection);
    }
    /// Starts or stops injecting the viewer's input on the selected local display.
    fn set_input(&self, enabled: bool) {
        if let Some(pump) = &self.pump {
            pump.set_input(enabled.then_some(self.monitor));
        }
    }
    fn stop(&mut self) {
        self.transport_ready = false;
        self.retry_at = None;
        self.retries = 0;
        self.finish_recording();
        self.audio_capture = None;
        self.audio_player = None;
        self.audio_failed = false;
        self.connecting = None;
        self.capture_errors = None;
        self.received_clipboard = None;
        self.capture_stop.store(true, Ordering::Relaxed);
        self.send(Command::Stop);
        self.connection = None;
        self.connected = false;
        self.control = false;
        self.incoming = None;
        self.pump = None;
        platform::stop_capture();
        self.texture = None;
        self.incoming_file = None;
        self.viewer_focused = false;
        self.pending_release = false;
        self.modifiers = egui::Modifiers::NONE;
        self.status = self
            .ui_text(
                "Durduruldu. Uzak erişim kapalı.",
                "Stopped. Remote access is closed.",
            )
            .into();
    }
    fn finish_recording(&mut self) {
        if let Some(mut recorder) = self.recorder.take()
            && let Err(e) = recorder.finish()
        {
            self.status = format!("Recording finalization failed: {e}");
        }
    }
    fn connect_host(&mut self) {
        if self.internet {
            match opendesk::p2p::host(&self.secret) {
                Ok(c) => {
                    self.attach(c);
                    self.hosting = true;
                    self.status =
                        "Sharing by code over the Internet. Nothing is shared until you accept."
                            .into();
                }
                Err(e) => {
                    self.status = e;
                    self.schedule_retry();
                }
            }
            return;
        }
        if self.relay && self.relay_address.trim().is_empty() {
            self.status = if self.settings.english {
                "Configure your relay server once in Settings for Internet connections."
            } else {
                "İnternet bağlantıları için kendi relay sunucunu Ayarlar'dan bir kez tanımla."
            }
            .into();
            return;
        }
        self.hosting = true;
        if self.relay {
            let (tx, rx) = std::sync::mpsc::channel();
            let address = self.relay_address.clone();
            let secret = self.secret.clone();
            self.connecting = Some(rx);
            self.connecting_host = true;
            self.status = "Connecting host to relay…".into();
            std::thread::spawn(move || {
                let result = network::host_relay(&address, &secret);
                if let Err(std::sync::mpsc::SendError(Ok(c))) = tx.send(result) {
                    let _ = c.commands.try_send(Command::Stop);
                }
            });
        } else {
            match network::host(&self.bind, &self.secret) {
                Ok(c) => {
                    self.attach(c);
                    self.hosting = true;
                    self.status = "Listening. No screen is shared until you accept.".into();
                }
                Err(e) => {
                    self.status = e;
                    self.schedule_retry();
                }
            }
        }
    }
    fn connect_viewer(&mut self) {
        if !self.internet && self.relay && self.relay_address.trim().is_empty() {
            self.status = if self.settings.english {
                "Configure your relay server once in Settings for Internet connections."
            } else {
                "İnternet bağlantıları için kendi relay sunucunu Ayarlar'dan bir kez tanımla."
            }
            .into();
            return;
        }
        self.hosting = false;
        let (tx, rx) = std::sync::mpsc::channel();
        let address = network::with_default_port(if self.relay {
            &self.relay_address
        } else {
            &self.address
        });
        if !self.internet && address.is_empty() {
            self.status = self
                .ui_text(
                    "Hedef bilgisayarın adresini (IP) yaz.",
                    "Enter the target computer's address (IP).",
                )
                .into();
            return;
        }
        let relay = self.relay;
        let internet = self.internet;
        let secret = self.secret.clone();
        self.connecting = Some(rx);
        self.connecting_host = false;
        self.status = "Connecting…".into();
        std::thread::spawn(move || {
            let result = if internet {
                opendesk::p2p::connect(&secret)
            } else if relay {
                network::connect_relay(&address, &secret)
            } else {
                network::connect(&address, &secret)
            };
            if let Err(std::sync::mpsc::SendError(Ok(c))) = tx.send(result) {
                let _ = c.commands.try_send(Command::Stop);
            }
        });
    }
    fn ui_text<'a>(&self, tr: &'a str, en: &'a str) -> &'a str {
        if self.settings.english { en } else { tr }
    }
    /// Address viewers on the same network use to reach this host.
    fn shared_address(&self) -> String {
        if self.relay {
            return self.relay_address.clone();
        }
        let port = self
            .bind
            .parse::<std::net::SocketAddr>()
            .map(|a| a.port())
            .unwrap_or(7878);
        match self.lan_ip {
            Some(ip) => std::net::SocketAddr::new(ip, port).to_string(),
            None => self.address.clone(),
        }
    }
    fn current_preferences(&self) -> settings::Preferences {
        settings::Preferences {
            auto_reconnect: self.auto_reconnect,
            adaptive: self.adaptive,
            quality: self.quality,
            fps: self.fps,
            max_width: self.max_width,
            bind: self.bind.clone(),
            inbox: self.inbox.clone(),
            record_path: self.record_path.clone(),
        }
    }
    /// Saves changed preferences once the user lets go of the slider or field.
    fn persist_preferences(&mut self, ctx: &egui::Context) {
        let current = self.current_preferences();
        if current != self.settings.preferences
            && !ctx.is_using_pointer()
            && ctx.memory(|m| m.focused().is_none())
        {
            self.settings.preferences = current;
            if let Err(e) = self.settings.save() {
                self.status = e;
            }
        }
    }
    fn schedule_retry(&mut self) {
        if let Some(delay) = retry_delay(self.auto_reconnect, self.retries) {
            self.retry_at = Some(std::time::Instant::now() + delay);
            self.retries += 1;
        }
    }
    fn extra_controls(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.collapsing("Signed application updates", |ui| {
            ui.label("No hosted update service is configured by default.");
            ui.label("Signed manifest URL");
            ui.text_edit_singleline(&mut self.settings.update_endpoint);
            if ui.button("Save update endpoint").clicked()
                && let Err(e) = self.settings.save()
            {
                self.status = e;
            }
            if !self.settings.update_endpoint.trim().is_empty()
                && self.update_work.is_none()
                && ui.button("Check for signed update").clicked()
            {
                let endpoint = self.settings.update_endpoint.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                self.update_work = Some(rx);
                self.status = "Checking signed update manifest…".into();
                std::thread::spawn(move || {
                    let _ = tx.send(opendesk::update::check(&endpoint).map(UpdateResult::Checked));
                });
            }
            if let Some(available) = &self.update_available {
                ui.label(format!(
                    "Signed release {} ({} bytes)",
                    available.manifest.version, available.manifest.size
                ));
            }
            if self.update_available.is_some()
                && self.update_work.is_none()
                && ui.button("Download and verify update").clicked()
            {
                let available = self.update_available.clone().unwrap();
                let (tx, rx) = std::sync::mpsc::channel();
                self.update_work = Some(rx);
                self.status = "Downloading and verifying update…".into();
                std::thread::spawn(move || {
                    let _ = tx
                        .send(opendesk::update::download(&available).map(UpdateResult::Downloaded));
                });
            }
            if self.staged_update.is_some()
                && ui.button("Apply verified update and restart").clicked()
            {
                let stage = self.staged_update.clone().unwrap();
                self.stop();
                match opendesk::update::apply(&stage) {
                    Ok(()) => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                    Err(e) => self.status = e,
                }
            }
            if self.update_work.is_some() {
                ui.spinner();
            }
        });
        ui.collapsing("Unattended access + authenticator",|ui|{
            ui.label("Host still runs visibly and must start listening. Saved credentials use the OS keyring.");
            ui.add(egui::TextEdit::singleline(&mut self.unattended_password).password(true).hint_text("Password (12+ characters)"));
            ui.text_edit_singleline(&mut self.unattended_token);
            if self.unattended.is_some(){ui.label("Unattended access enabled; password + fresh TOTP required");if ui.button("Disable unattended access").clicked(){match opendesk::unattended::disable(){Ok(())=>self.unattended=None,Err(e)=>self.status=e}}}else{
                if ui.button("Enroll authenticator").clicked(){match opendesk::unattended::begin_enrollment(&self.unattended_password,&self.secret,self.permissions){Ok(e)=>self.enrollment=Some(e),Err(e)=>self.status=e}}
                if let Some(enrollment)=&self.enrollment{ui.label("Enter this secret in your authenticator, then confirm its current code:");ui.monospace(&enrollment.secret_base32);if ui.button("Copy authenticator enrollment URI").clicked(){ctx.copy_text(enrollment.otpauth_uri.clone());}}
                if self.enrollment.is_some() && ui.button("Confirm authenticator code").clicked(){match opendesk::unattended::confirm_enrollment(&self.unattended_token){Ok(policy)=>{self.unattended=Some(policy);self.enrollment=None;self.unattended_password.clear();self.unattended_token.clear();},Err(e)=>self.status=e}}
            }
            ui.checkbox(&mut self.use_unattended,"Viewer: send password + authenticator code when connecting");
        });
        ui.collapsing("Invite and saved devices", |ui| {
            ui.label("Private invitation (contains pairing secret)");
            ui.text_edit_singleline(&mut self.invite);
            if ui.button("Copy invitation").clicked() {
                let endpoint = self.shared_address();
                match opendesk::invite::encode(&endpoint, self.relay, &self.secret) {
                    Ok(code) => {
                        self.invite = code.clone();
                        ctx.copy_text(code);
                    }
                    Err(e) => self.status = e,
                }
            }
            if ui.button("Load invitation").clicked() {
                match opendesk::invite::decode(&self.invite) {
                    Ok(invite) => {
                        self.internet = false;
                        self.relay = invite.relay;
                        if invite.relay {
                            self.relay_address = invite.endpoint
                        } else {
                            self.address = invite.endpoint
                        }
                        self.secret = invite.secret;
                    }
                    Err(e) => self.status = e,
                }
            }
            ui.label("Device nickname");
            ui.text_edit_singleline(&mut self.device_name);
            if ui.button("Save address (no secret)").clicked()
                && !self.device_name.trim().is_empty()
            {
                let address = if self.relay {
                    self.relay_address.clone()
                } else {
                    self.address.clone()
                };
                self.settings.devices.push(settings::Device {
                    name: self.device_name.clone(),
                    address,
                    relay: self.relay,
                    favorite: false,
                });
                if let Err(e) = self.settings.save() {
                    self.status = e;
                }
            }
            let mut selected = None;
            let mut remove = None;
            let mut changed = false;
            for (i, device) in self.settings.devices.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    changed |= ui.checkbox(&mut device.favorite, "★").changed();
                    if ui
                        .button(format!("{} — {}", device.name, device.address))
                        .clicked()
                    {
                        selected = Some(device.clone());
                    }
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.settings.devices.remove(i);
                changed = true;
            }
            if changed && let Err(e) = self.settings.save() {
                self.status = e;
            }
            if let Some(d) = selected {
                self.relay = d.relay;
                if d.relay {
                    self.relay_address = d.address
                } else {
                    self.address = d.address
                }
            }
        });
        ui.checkbox(
            &mut self.auto_reconnect,
            "Reconnect / relisten after network loss (5 attempts)",
        );
        if !self.auto_reconnect {
            self.retry_at = None;
        }
        if self.retry_at.is_some() {
            ui.label(format!(
                "Reconnect attempt {} / 5; STOP cancels",
                self.retries
            ));
        }
        ui.checkbox(&mut self.adaptive, "Adapt video quality to received frames");
        ui.label(&self.video_stats);
        ui.label(format!("Video {:.1} FPS", self.actual_fps));
        ui.add(egui::Slider::new(&mut self.max_width, 640..=3840).text("Maximum video width"));
        let before = self.permissions;
        ui.collapsing("Host grants / live permissions", |ui| {
            ui.add_enabled_ui(self.hosting || !self.connected, |ui| {
                ui.checkbox(&mut self.permissions.screen, "Screen");
                ui.checkbox(&mut self.permissions.input, "Mouse and keyboard");
                ui.checkbox(&mut self.permissions.clipboard, "Clipboard");
                ui.checkbox(&mut self.permissions.files, "Files");
                ui.checkbox(&mut self.permissions.audio, "System audio");
            });
        });
        if before != self.permissions && self.hosting && self.connected {
            if !self.send(Command::SetPermissions(self.permissions)) {
                self.permissions = before;
                self.status = "Permission queue busy. Try again.".into();
            } else {
                self.set_input(self.permissions.input);
            }
        }
        if self.connected && !self.hosting {
            let mut selected = None;
            egui::ComboBox::from_label("Remote monitor")
                .selected_text(format!("Display {}", self.monitor + 1))
                .show_ui(ui, |ui| {
                    for monitor in &self.monitors {
                        if ui
                            .selectable_label(
                                self.monitor == monitor.id,
                                format!("{} ({}×{})", monitor.name, monitor.width, monitor.height),
                            )
                            .clicked()
                        {
                            selected = Some(monitor.id);
                        }
                    }
                });
            if let Some(id) = selected
                && self.send(Command::SelectMonitor(id))
            {
                self.finish_recording();
                self.monitor = id;
            }
            ui.label("Video recording (.avi, Motion JPEG)");
            ui.text_edit_singleline(&mut self.record_path);
            if self.recorder.is_some() {
                ui.colored_label(egui::Color32::RED, "● RECORDING");
                if ui.button("Stop recording").clicked() {
                    self.finish_recording();
                }
            } else if ui.button("Start recording").clicked() {
                if let Some((w, h)) = self.frame_size {
                    match recording::Recorder::start(
                        std::path::Path::new(&self.record_path),
                        w,
                        h,
                        self.fps as u32,
                    ) {
                        Ok(r) => self.recorder = Some(r),
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status = "Wait for a remote video frame before recording".into();
                }
            }
        }
    }
    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.update_work.as_ref().and_then(|r| r.try_recv().ok()) {
            self.update_work = None;
            match result {
                Ok(UpdateResult::Checked(available)) => {
                    self.update_available = Some(available);
                    self.status =
                        "Update signature verified. Download requires your button click.".into();
                }
                Ok(UpdateResult::Downloaded(path)) => {
                    self.staged_update = Some(path);
                    self.status =
                        "Update downloaded and verified. Apply when ready to restart.".into();
                }
                Err(e) => self.status = format!("Update: {e}"),
            }
        }
        if self
            .retry_at
            .is_some_and(|t| std::time::Instant::now() >= t)
            && self.connection.is_none()
            && self.connecting.is_none()
            && (self.hosting || !self.use_unattended || !self.unattended_token.is_empty())
        {
            self.retry_at = None;
            if self.hosting {
                self.connect_host();
            } else {
                self.connect_viewer();
            }
        }
        if let Ok(mut capture) = self.capture_settings.lock() {
            *capture = (
                self.monitor,
                self.quality,
                self.fps,
                self.permissions.screen,
                self.max_width,
            );
        }
        if self.hosting && self.connected && self.permissions.audio && self.audio_capture.is_none()
        {
            match audio::AudioCapture::start() {
                Ok(a) => self.audio_capture = Some(a),
                Err(e) => {
                    self.permissions.audio = false;
                    self.send(Command::SetPermissions(self.permissions));
                    self.status = format!("Audio capture unavailable: {e}");
                }
            }
        }
        if !self.permissions.audio {
            self.audio_capture = None;
            self.audio_player = None;
        }
        if let Some(error) = self
            .audio_capture
            .as_ref()
            .and_then(audio::AudioCapture::try_error)
        {
            self.permissions.audio = false;
            self.audio_capture = None;
            self.send(Command::SetPermissions(self.permissions));
            self.status = format!("System audio stopped: {error}");
        }
        let mut packets = Vec::new();
        if let Some(capture) = &self.audio_capture {
            while let Some(p) = capture.try_recv() {
                packets.push(p);
            }
        }
        for packet in packets {
            self.send(Command::Audio {
                sample_rate: packet.sample_rate,
                channels: packet.channels,
                pcm: packet.pcm,
            });
        }
        if let Some(result) = self.connecting.as_ref().and_then(|r| r.try_recv().ok()) {
            self.connecting = None;
            match result {
                Ok(c) => {
                    if self.use_unattended && !self.connecting_host {
                        let _ = c.commands.try_send(Command::Authenticate {
                            password: self.unattended_password.clone(),
                            totp: self.unattended_token.clone(),
                        });
                        self.unattended_token.clear();
                    }
                    self.attach(c);
                    self.hosting = self.connecting_host;
                    self.status = "Waiting for connection / acceptance".into();
                }
                Err(e) => {
                    self.status = e;
                    self.schedule_retry();
                }
            }
        }
        if let Some(error) = self.capture_errors.as_ref().and_then(|r| r.try_recv().ok()) {
            self.stop();
            self.status = format!("Capture failed: {error}");
        }
        let mut events = Vec::new();
        if let Some(c) = &self.connection {
            loop {
                match c.events.try_recv() {
                    Ok(event) => events.push(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        if !events.iter().any(|e| matches!(e, Event::Disconnected)) {
                            events.push(Event::Disconnected);
                        }
                        break;
                    }
                }
            }
        }
        for event in events {
            match event {
                Event::Status(text) => {
                    if text == "Waiting for host approval" {
                        self.transport_ready = true;
                    }
                    self.status = text;
                }
                Event::Incoming { peer } => {
                    self.incoming = Some(peer);
                    self.status = "Connection request: review and accept locally.".into();
                }
                Event::Authentication { password, totp } => {
                    if self.hosting {
                        match self
                            .unattended
                            .as_mut()
                            .ok_or_else(|| "Unattended access is disabled".to_owned())
                            .and_then(|policy| policy.verify(&password, &totp))
                        {
                            Ok(permissions) => {
                                self.permissions = permissions;
                                self.send(Command::AcceptPermissions(permissions));
                            }
                            Err(e) => {
                                self.status = e;
                                self.send(Command::Reject);
                            }
                        }
                    }
                }
                Event::Connected { peer, control } => {
                    self.transport_ready = true;
                    self.peer_label = peer.clone();
                    self.retries = 0;
                    self.retry_at = None;
                    self.connected = true;
                    self.control = control;
                    self.incoming = None;
                    self.status = format!(
                        "Connected: {peer} | {}",
                        if control {
                            "screen + control"
                        } else {
                            "view only"
                        }
                    );
                    if self.hosting {
                        platform::start_capture();
                        if let Ok(monitors) = platform::display_list() {
                            self.send(Command::Monitors(monitors));
                        }
                        self.set_input(control);
                        let Some(tx) = self.connection.as_ref().map(|c| c.commands.clone()) else {
                            continue;
                        };
                        self.capture_stop = Arc::new(AtomicBool::new(false));
                        let stop = self.capture_stop.clone();
                        let capture_settings = self.capture_settings.clone();
                        let (error_tx, error_rx) = std::sync::mpsc::sync_channel(1);
                        self.capture_errors = Some(error_rx);
                        std::thread::spawn(move || {
                            // Lock screens, UAC prompts and display changes make capture
                            // fail briefly; only a sustained failure ends the session.
                            let mut failures = 0u32;
                            while !stop.load(Ordering::Relaxed) {
                                let (monitor, quality, fps, sharing, max_width) =
                                    *capture_settings.lock().unwrap_or_else(|e| e.into_inner());
                                let delay = 1000 / fps.max(1);
                                if !sharing {
                                    std::thread::sleep(Duration::from_millis(100));
                                    continue;
                                }
                                match platform::capture_display_scaled(monitor, quality, max_width)
                                {
                                    Ok(jpeg) => {
                                        failures = 0;
                                        match tx.try_send(Command::Frame(jpeg)) {
                                            Ok(())
                                            | Err(std::sync::mpsc::TrySendError::Full(_)) => {}
                                            Err(_) => break,
                                        }
                                    }
                                    Err(_) if failures < MAX_CAPTURE_FAILURES => {
                                        failures += 1;
                                        std::thread::sleep(Duration::from_millis(250));
                                        continue;
                                    }
                                    Err(error) => {
                                        let _ = error_tx.try_send(error);
                                        let _ = tx.try_send(Command::Stop);
                                        break;
                                    }
                                }
                                std::thread::sleep(Duration::from_millis(delay));
                            }
                        });
                    }
                }
                Event::Frame { .. } => {}
                Event::IncomingFile { name, bytes } => {
                    self.incoming_file = Some((name, bytes));
                }
                Event::TransferOffer(manifest) => self.transfer_offer = Some(manifest),
                Event::TransferProgress {
                    id,
                    completed,
                    total,
                    receiving,
                } => {
                    self.transfer_status = format!(
                        "{} {}: {:.1}% ({completed}/{total} bytes)",
                        if receiving { "Receiving" } else { "Sending" },
                        id,
                        completed as f64 * 100.0 / (total.max(1) as f64)
                    )
                }
                Event::TransferComplete { id } => {
                    self.transfer_status = format!("Transfer {id} completed")
                }
                Event::Chat(text) => {
                    self.messages.push(format!("Peer: {text}"));
                    if self.messages.len() > 100 {
                        self.messages.remove(0);
                    }
                }
                // Injected by the pump thread so control keeps working while minimized.
                Event::Input(_) => {}
                Event::Clipboard(text) => self.received_clipboard = Some(text),
                Event::Permissions(p) => {
                    self.permissions = p;
                    if !p.screen {
                        self.texture = None;
                        self.frame_size = None;
                        self.finish_recording();
                    }
                    if !p.clipboard {
                        self.received_clipboard = None;
                    }
                    if !p.files {
                        self.transfer_offer = None;
                        self.incoming_file = None;
                    }
                    self.control = p.input;
                    if self.hosting {
                        self.set_input(p.input);
                    }
                }
                Event::Monitors(monitors) => self.monitors = monitors,
                Event::SelectMonitor(index) => {
                    self.finish_recording();
                    self.monitor = index;
                    if self.hosting {
                        self.set_input(self.control);
                    }
                }
                Event::VideoFeedback { max_fps, quality } => {
                    if self.hosting && self.adaptive {
                        self.fps = u64::from(max_fps).clamp(1, 30);
                        self.quality = quality.clamp(30, 90);
                    }
                }
                Event::Metrics {
                    sent_bytes,
                    received_bytes,
                } => {
                    self.video_stats = format!(
                        "Sent {:.1} MB • received {:.1} MB",
                        sent_bytes as f64 / 1_000_000.0,
                        received_bytes as f64 / 1_000_000.0
                    )
                }
                Event::Audio {
                    sample_rate,
                    channels,
                    pcm,
                } => {
                    if !self.hosting && self.permissions.audio && !self.audio_failed {
                        if self.audio_player.is_none() {
                            match audio::AudioPlayer::new() {
                                Ok(player) => self.audio_player = Some(player),
                                Err(e) => {
                                    self.audio_failed = true;
                                    self.status = format!("Audio playback unavailable: {e}");
                                }
                            }
                        }
                        if let Some(player) = &mut self.audio_player
                            && let Err(e) = player.push(audio::AudioPacket {
                                sample_rate,
                                channels,
                                pcm,
                            })
                        {
                            self.status = e;
                        }
                    }
                }
                Event::Disconnected => {
                    let status = self.status.clone();
                    let retries = self.retries;
                    self.stop();
                    self.retries = retries;
                    self.schedule_retry();
                    self.status = format!("Session closed. {status}");
                }
            }
        }
        self.take_frame(ctx);
        if self.feedback_at.elapsed() >= Duration::from_secs(2) {
            self.actual_fps = self.frame_window as f32 / self.feedback_at.elapsed().as_secs_f32();
            if self.connected && !self.hosting && self.adaptive {
                let actual = self.frame_window as f32 / self.feedback_at.elapsed().as_secs_f32();
                let quality = if actual < self.fps as f32 * 0.7 {
                    self.quality.saturating_sub(5).max(30)
                } else {
                    self.quality.saturating_add(2).min(85)
                };
                let fps = if self.bytes_window > 3_000_000 {
                    self.fps.saturating_sub(1).max(2)
                } else {
                    self.fps.min(15)
                };
                self.send(Command::VideoFeedback {
                    max_fps: fps as u8,
                    quality,
                });
                self.quality = quality;
            }
            self.frame_window = 0;
            self.bytes_window = 0;
            self.feedback_at = std::time::Instant::now();
        }
    }
}
impl Desk {
    fn take_frame(&mut self, ctx: &egui::Context) {
        let Some(pump) = &self.pump else { return };
        let (latest, frames, bytes) = {
            let mut slot = pump.frames.lock().unwrap_or_else(|e| e.into_inner());
            (
                slot.latest.take(),
                std::mem::take(&mut slot.frames),
                std::mem::take(&mut slot.bytes),
            )
        };
        self.frame_window += frames;
        self.bytes_window += bytes;
        let Some((jpeg, color)) = latest else { return };
        if !self.permissions.screen {
            return;
        }
        let (w, h) = (color.size[0] as u32, color.size[1] as u32);
        self.frame_size = Some((w, h));
        if let Some(recorder) = &mut self.recorder
            && let Err(e) = recorder.realtime_frame(&jpeg, w, h)
        {
            self.status = e;
            self.finish_recording();
        }
        if let Some(t) = &mut self.texture {
            t.set(color, egui::TextureOptions::LINEAR);
        } else {
            self.texture = Some(ctx.load_texture("remote", color, egui::TextureOptions::LINEAR));
        }
    }
    fn remote_canvas(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if let Some(texture) = self.texture.clone() {
            let size = texture.size_vec2();
            let response = if self.fit_to_window {
                // Scale to the window in both directions and center, keeping aspect ratio.
                let available = ui.available_size();
                let scale = (available.x / size.x).min(available.y / size.y).max(0.05);
                let (area, _) = ui.allocate_exact_size(available, egui::Sense::hover());
                ui.put(
                    egui::Rect::from_center_size(area.center(), size * scale),
                    egui::Image::new(&texture)
                        .fit_to_exact_size(size * scale)
                        .sense(egui::Sense::click_and_drag()),
                )
            } else {
                egui::ScrollArea::both()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add(
                            egui::Image::new(&texture)
                                .fit_to_exact_size(size)
                                .sense(egui::Sense::click_and_drag()),
                        )
                    })
                    .inner
            };
            if self.control && self.connected && !self.hosting {
                if response.hovered() {
                    response.request_focus();
                }
                if ctx.input(|i| i.pointer.delta() != egui::Vec2::ZERO)
                    && let Some(pos) = response.hover_pos()
                {
                    let rel = (pos - response.rect.min) / response.rect.size();
                    self.send(Command::Input(InputEvent::Move { x: rel.x, y: rel.y }));
                }
                let focused = response.has_focus() && ctx.input(|i| i.focused);
                if self.viewer_focused && !focused {
                    self.pending_release = true;
                }
                if self.pending_release && self.send(Command::Input(InputEvent::ReleaseAll)) {
                    self.pending_release = false;
                }
                self.viewer_focused = focused;
                let final_modifiers = if focused {
                    ctx.input(|i| i.modifiers)
                } else {
                    egui::Modifiers::NONE
                };
                let events = if focused {
                    ctx.input(|i| i.events.clone())
                } else {
                    Vec::new()
                };
                for input in translate_remote_events(
                    &events,
                    self.modifiers,
                    final_modifiers,
                    response.hovered(),
                ) {
                    if !self.send(Command::Input(input)) {
                        self.pending_release = true;
                    }
                }
                self.modifiers = final_modifiers;
            }
        } else {
            let text = if self.hosting && self.connected {
                self.ui_text(
                    "Ekran paylaşımı açık. Erişimi kapatmak için Bitir'e bas.",
                    "Screen sharing active. Press End to revoke access.",
                )
            } else if self.connected && !self.permissions.screen {
                self.ui_text(
                    "Karşı taraf ekran paylaşımını kapattı.",
                    "The host turned off screen sharing.",
                )
            } else {
                self.ui_text("Görüntü bekleniyor…", "Waiting for the remote screen…")
            };
            ui.centered_and_justified(|ui| {
                ui.label(text);
            });
        }
    }
}
impl eframe::App for Desk {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        if self.smoke {
            self.smoke_frames += 1;
            if self.smoke_frames == 2 {
                println!("GUI initialized");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            ctx.request_repaint();
            return;
        }
        self.poll(ctx);
        self.poll_class(ctx);
        ctx.request_repaint_after(Duration::from_millis(50));
        self.design_ui(ctx);
        self.persist_preferences(ctx);
        if let Some(path) = &self.ui_capture {
            for event in ctx.input(|i| i.events.clone()) {
                if let egui::Event::Screenshot { image, .. } = event {
                    let bytes: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
                    let result = image::save_buffer(
                        path,
                        &bytes,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ColorType::Rgba8,
                    );
                    if let Err(e) = result {
                        eprintln!("UI capture failed: {e}");
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
            self.ui_capture_frames = self.ui_capture_frames.saturating_add(1);
            if self.ui_capture_frames == 4 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(())));
            }
        }
    }
}
impl Drop for Desk {
    fn drop(&mut self) {
        let current = self.current_preferences();
        if current != self.settings.preferences {
            self.settings.preferences = current;
            let _ = self.settings.save();
        }
        self.stop();
    }
}

fn decode_frame(jpeg: &[u8]) -> Result<image::DynamicImage, String> {
    let reader = image::ImageReader::new(std::io::Cursor::new(jpeg))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if w == 0 || h == 0 || w > 8192 || h > 8192 || (w as u64) * (h as u64) > 32_000_000 {
        return Err("Display image exceeds dimensions limit".into());
    }
    let mut decoder =
        image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(128 * 1024 * 1024);
    decoder.limits(limits);
    decoder.decode().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::decode_frame;
    fn key_event(
        key: eframe::egui::Key,
        pressed: bool,
        modifiers: eframe::egui::Modifiers,
    ) -> eframe::egui::Event {
        eframe::egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        }
    }
    #[test]
    fn rapid_ctrl_s_keeps_event_order_when_frame_modifiers_are_released() {
        use super::InputEvent;
        use eframe::egui::{Event, Key, Modifiers};
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let events = [
            key_event(Key::S, true, ctrl),
            Event::Text("s".into()),
            key_event(Key::S, false, ctrl),
        ];
        let output =
            super::translate_remote_events(&events, Modifiers::NONE, Modifiers::NONE, true);
        let keys: Vec<_> = output
            .iter()
            .map(|event| match event {
                InputEvent::Key { key, pressed } => (key.as_str(), *pressed),
                _ => panic!("Shortcut emitted duplicate text"),
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                ("Control", true),
                ("S", true),
                ("S", false),
                ("Control", false)
            ]
        );
    }
    #[test]
    fn native_paste_sends_text_once_without_remote_ctrl_v() {
        use eframe::egui::{Event, Key, Modifiers};
        let ctrl = Modifiers {
            ctrl: true,
            command: true,
            ..Modifiers::NONE
        };
        let text = "Merhaba, hızlı yapıştırma! 🦀";
        let events = [
            key_event(Key::V, true, ctrl),
            Event::Paste(text.into()),
            key_event(Key::V, false, ctrl),
        ];
        let output =
            super::translate_remote_events(&events, Modifiers::NONE, Modifiers::NONE, true);
        assert_eq!(output.len(), 1);
        assert!(matches!(&output[0],super::InputEvent::Text(value)if value==text));
    }
    #[test]
    fn paste_chunks_preserve_utf8_and_protocol_text_bound() {
        use eframe::egui::{Event, Modifiers};
        let text = "ü".repeat(4097);
        let output = super::translate_remote_events(
            &[Event::Paste(text.clone())],
            Modifiers::NONE,
            Modifiers::NONE,
            true,
        );
        let chunks: Vec<_> = output
            .iter()
            .map(|event| match event {
                super::InputEvent::Text(chunk) => {
                    assert!(chunk.len() <= 4096);
                    chunk.as_str()
                }
                _ => panic!("Unexpected paste event"),
            })
            .collect();
        assert_eq!(chunks.concat(), text);
    }
    #[test]
    fn ordinary_digit_text_has_no_duplicate_physical_press() {
        use eframe::egui::{Event, Key, Modifiers};
        let output = super::translate_remote_events(
            &[
                key_event(Key::Num1, true, Modifiers::NONE),
                Event::Text("1".into()),
                key_event(Key::Num1, false, Modifiers::NONE),
            ],
            Modifiers::NONE,
            Modifiers::NONE,
            true,
        );
        assert!(
            !output
                .iter()
                .any(|event| matches!(event, super::InputEvent::Key { pressed: true, .. }))
        );
        assert_eq!(
            output
                .iter()
                .filter(|event| matches!(event,super::InputEvent::Text(text)if text=="1"))
                .count(),
            1
        );
    }
    #[test]
    fn reconnect_backoff_is_bounded_and_cancelable() {
        assert_eq!(
            super::retry_delay(true, 0),
            Some(std::time::Duration::from_secs(1))
        );
        assert_eq!(
            super::retry_delay(true, 4),
            Some(std::time::Duration::from_secs(16))
        );
        assert_eq!(super::retry_delay(true, 5), None);
        assert_eq!(super::retry_delay(true, u8::MAX), None);
        assert_eq!(super::retry_delay(false, 0), None);
    }
    #[test]
    fn rejects_malformed_remote_image() {
        assert!(decode_frame(b"invalid JPEG").is_err());
    }
    #[test]
    fn rejects_oversized_remote_image() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&vec![0; 8193 * 3], 8193, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        assert!(decode_frame(&jpeg).is_err());
    }
    #[test]
    fn accepts_valid_remote_image() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(&[255, 0, 0], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let image = decode_frame(&jpeg).unwrap();
        assert_eq!((image.width(), image.height()), (1, 1));
    }
}
