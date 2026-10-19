//! Classroom mode: the teacher's class view and the student's side of a lesson.
use crate::design::{Palette, button, card, label};
use crate::{Desk, decode_frame, platform, settings::Role, translate_remote_events};
use eframe::egui::{self, Color32, RichText, Vec2};
use opendesk::classroom::{
    self, Announced, ClassServer, LinkEvent, StudentLink, TeacherLink, ToStudent, ToTeacher,
};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    time::{Duration, Instant},
};

/// Capture settings (fps, JPEG quality, maximum width) a student uses for each view.
type CaptureMode = (u8, u8, u32);
const THUMBNAIL: CaptureMode = (1, 40, 480);
const FOCUSED: CaptureMode = (8, 60, 1600);
const SHOWN: CaptureMode = (6, 60, 1600);
const RETRY_EVERY: Duration = Duration::from_secs(3);

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Opendesk".into())
}
fn user_name() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}

/// Opens a teacher-sent web address in the default browser (http/https only).
fn open_url(url: &str) -> Result<(), String> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://"))
        || url.len() > 2048
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"')
    {
        return Err("Only http(s) web addresses can be opened".into());
    }
    let mut command = if cfg!(windows) {
        let mut c = std::process::Command::new("rundll32");
        c.args(["url.dll,FileProtocolHandler", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

fn upload(ctx: &egui::Context, texture: &mut Option<egui::TextureHandle>, name: &str, jpeg: &[u8]) {
    let Ok(image) = decode_frame(jpeg) else {
        return;
    };
    let rgba = image.to_rgba8();
    let color = egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        &rgba,
    );
    match texture {
        Some(t) => t.set(color, egui::TextureOptions::LINEAR),
        None => *texture = Some(ctx.load_texture(name, color, egui::TextureOptions::LINEAR)),
    }
}

/// Draws a texture scaled to fit `size`, centered, and returns its response.
fn fitted_image(
    ui: &mut egui::Ui,
    texture: &egui::TextureHandle,
    size: Vec2,
    sense: egui::Sense,
) -> egui::Response {
    let image = texture.size_vec2();
    let scale = (size.x / image.x).min(size.y / image.y).max(0.01);
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let shown = egui::Rect::from_center_size(rect.center(), image * scale);
    ui.put(
        shown,
        egui::Image::new(texture)
            .fit_to_exact_size(image * scale)
            .sense(sense),
    )
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Show {
    #[default]
    Nothing,
    Teacher,
    Student(u64),
}

struct Pupil {
    id: u64,
    name: String,
    computer: String,
    link: Arc<StudentLink>,
    texture: Option<egui::TextureHandle>,
    hand: bool,
    locked: bool,
    selected: bool,
    last_frame: Option<Instant>,
    mode: CaptureMode,
}

#[derive(Default)]
pub struct Teacher {
    server: Option<ClassServer>,
    pupils: Vec<Pupil>,
    next_id: u64,
    focus: Option<u64>,
    control: bool,
    show: Show,
    presenting: Option<Arc<AtomicBool>>,
    audience: Arc<Mutex<Vec<Arc<StudentLink>>>>,
    all_locked: bool,
    message: String,
    url: String,
    modifiers: egui::Modifiers,
    inbox: Vec<String>,
    controlling: bool,
}

struct Joined {
    link: Arc<TeacherLink>,
    class: String,
    capture: Arc<Mutex<CaptureMode>>,
    stop: Arc<AtomicBool>,
    input: Option<platform::InputController>,
}
impl Drop for Joined {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.link.send(ToTeacher::Bye);
    }
}

#[derive(Default)]
pub struct Pupilside {
    joined: Option<Joined>,
    joining: Option<Receiver<Result<TeacherLink, String>>>,
    discovery: Option<classroom::Discovery>,
    found: Vec<Announced>,
    show: Option<String>,
    show_texture: Option<egui::TextureHandle>,
    lock: Option<String>,
    popup: Option<String>,
    hand: bool,
    message: String,
    messages: Vec<String>,
    /// The student asked to be in the class; reconnect when the link drops.
    wants: bool,
    retry_at: Option<Instant>,
    fullscreen_applied: bool,
    discovery_retry: Option<Instant>,
}

#[derive(Default)]
pub struct Classroom {
    pub teacher: Teacher,
    pub student: Pupilside,
    pub status: String,
    autojoin_checked: bool,
}

impl Classroom {
    pub fn teacher_active(&self) -> bool {
        self.teacher.server.is_some()
    }
    pub fn student_active(&self) -> bool {
        self.student.joined.is_some()
    }
}

impl Desk {
    fn class_status(&mut self, tr: &str, en: &str) {
        self.class.status = self.tr(tr, en).to_owned();
    }

    // ---------------------------------------------------------------- teacher

    fn start_class(&mut self) {
        let prefs = &mut self.settings.class;
        if prefs.class_name.trim().is_empty() {
            prefs.class_name = format!("{} sınıfı", computer_name());
        }
        if classroom::normalize_code(&prefs.class_code).is_none() {
            prefs.class_code = classroom::generate_class_code();
        }
        match classroom::serve(classroom::CLASS_PORT, &prefs.class_code, &prefs.class_name) {
            Ok(server) => {
                self.class.teacher.server = Some(server);
                self.class_status(
                    "Sınıf açık. Öğrenciler Sınıf > Öğrenci ekranından katılabilir.",
                    "Class is open. Students join from Class > Student.",
                );
                self.persist_class();
            }
            Err(e) => self.class.status = e,
        }
    }
    fn persist_class(&mut self) {
        if let Err(e) = self.settings.save() {
            self.status = e;
        }
    }
    fn end_class(&mut self) {
        self.stop_show();
        let teacher = &mut self.class.teacher;
        for pupil in &teacher.pupils {
            pupil.link.send(ToStudent::Bye);
        }
        teacher.pupils.clear();
        teacher
            .audience
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        teacher.server = None;
        teacher.focus = None;
        teacher.all_locked = false;
        self.class_status("Sınıf kapatıldı.", "Class closed.");
    }
    fn wanted_mode(&self, id: u64) -> CaptureMode {
        let teacher = &self.class.teacher;
        if teacher.show == Show::Student(id) {
            SHOWN
        } else if teacher.focus == Some(id) {
            FOCUSED
        } else {
            THUMBNAIL
        }
    }
    fn update_capture_modes(&mut self) {
        let modes: Vec<_> = self
            .class
            .teacher
            .pupils
            .iter()
            .map(|p| self.wanted_mode(p.id))
            .collect();
        for (pupil, mode) in self.class.teacher.pupils.iter_mut().zip(modes) {
            if pupil.mode != mode
                && pupil.link.send(ToStudent::Capture {
                    fps: mode.0,
                    quality: mode.1,
                    max_width: mode.2,
                })
            {
                pupil.mode = mode;
            }
        }
    }
    fn present_mine(&mut self) {
        self.stop_show();
        let stop = Arc::new(AtomicBool::new(false));
        let audience = self.class.teacher.audience.clone();
        {
            let stop = stop.clone();
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let started = Instant::now();
                    if let Ok(jpeg) = platform::capture_display_scaled(0, 60, 1600) {
                        let jpeg = Arc::new(jpeg);
                        for link in audience.lock().unwrap_or_else(|e| e.into_inner()).iter() {
                            if !link.frame_pending() {
                                link.send_frame(jpeg.clone());
                            }
                        }
                    }
                    std::thread::sleep(
                        Duration::from_millis(200).saturating_sub(started.elapsed()),
                    );
                }
            });
        }
        let title = self.tr("Öğretmenin ekranı", "Teacher's screen").to_owned();
        for pupil in &self.class.teacher.pupils {
            pupil.link.send(ToStudent::ShowStart {
                title: title.clone(),
            });
        }
        self.class.teacher.presenting = Some(stop);
        self.class.teacher.show = Show::Teacher;
    }
    fn present_student(&mut self, id: u64) {
        self.stop_show();
        let Some(name) = self
            .class
            .teacher
            .pupils
            .iter()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
        else {
            return;
        };
        for pupil in self.class.teacher.pupils.iter().filter(|p| p.id != id) {
            pupil.link.send(ToStudent::ShowStart {
                title: name.clone(),
            });
        }
        self.class.teacher.show = Show::Student(id);
        self.update_capture_modes();
    }
    fn stop_show(&mut self) {
        let teacher = &mut self.class.teacher;
        if let Some(stop) = teacher.presenting.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if teacher.show != Show::Nothing {
            for pupil in &teacher.pupils {
                pupil.link.send(ToStudent::ShowEnd);
            }
        }
        teacher.show = Show::Nothing;
        self.update_capture_modes();
    }
    /// Selected students, or the whole class when nobody is selected.
    fn targets(&self) -> Vec<u64> {
        let pupils = &self.class.teacher.pupils;
        let selected: Vec<_> = pupils.iter().filter(|p| p.selected).map(|p| p.id).collect();
        if selected.is_empty() {
            pupils.iter().map(|p| p.id).collect()
        } else {
            selected
        }
    }
    fn lock(&mut self, locked: bool) {
        let targets = self.targets();
        let everyone = targets.len() == self.class.teacher.pupils.len();
        let message = self
            .tr(
                "Lütfen öğretmeni dinleyin.",
                "Please pay attention to the teacher.",
            )
            .to_owned();
        for pupil in self
            .class
            .teacher
            .pupils
            .iter_mut()
            .filter(|p| targets.contains(&p.id))
        {
            if pupil.link.send(ToStudent::Lock {
                locked,
                message: message.clone(),
            }) {
                pupil.locked = locked;
            }
        }
        if everyone {
            self.class.teacher.all_locked = locked;
        }
    }
    fn broadcast(&mut self, message: ToStudent) {
        let targets = self.targets();
        for pupil in self
            .class
            .teacher
            .pupils
            .iter()
            .filter(|p| targets.contains(&p.id))
        {
            pupil.link.send(message.clone());
        }
    }
    fn set_control(&mut self, enabled: bool) {
        let teacher = &mut self.class.teacher;
        if let Some(pupil) = teacher.pupils.iter().find(|p| Some(p.id) == teacher.focus) {
            pupil.link.send(ToStudent::Control(enabled));
        }
        teacher.control = enabled;
    }
    fn set_focus(&mut self, focus: Option<u64>) {
        if self.class.teacher.control {
            self.set_control(false);
        }
        self.class.teacher.focus = focus;
        self.update_capture_modes();
    }

    fn poll_teacher(&mut self, ctx: &egui::Context) {
        let mut joined = Vec::new();
        if let Some(server) = &self.class.teacher.server {
            joined.extend(server.joined.try_iter());
        }
        for link in joined {
            let link = Arc::new(link);
            let teacher = &mut self.class.teacher;
            teacher.next_id += 1;
            link.send(ToStudent::Welcome {
                class: self.settings.class.class_name.clone(),
            });
            link.send(ToStudent::Capture {
                fps: THUMBNAIL.0,
                quality: THUMBNAIL.1,
                max_width: THUMBNAIL.2,
            });
            if teacher.all_locked {
                link.send(ToStudent::Lock {
                    locked: true,
                    message: String::new(),
                });
            }
            if teacher.show != Show::Nothing {
                link.send(ToStudent::ShowStart {
                    title: String::new(),
                });
            }
            teacher
                .audience
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(link.clone());
            let all_locked = teacher.all_locked;
            teacher.pupils.push(Pupil {
                id: teacher.next_id,
                name: link.peer.ip().to_string(),
                computer: String::new(),
                link,
                texture: None,
                hand: false,
                locked: all_locked,
                selected: false,
                last_frame: None,
                mode: THUMBNAIL,
            });
        }
        let mut closed = Vec::new();
        let mut forward = None;
        let show = self.class.teacher.show;
        for pupil in &mut self.class.teacher.pupils {
            let mut latest = None;
            for event in pupil.link.poll_events() {
                match event {
                    LinkEvent::Message(ToTeacher::Hello { name, computer }) => {
                        pupil.name = name.chars().take(48).collect();
                        pupil.computer = computer.chars().take(64).collect();
                    }
                    LinkEvent::Message(ToTeacher::Hand(up)) => pupil.hand = up,
                    LinkEvent::Message(ToTeacher::Message(text)) => {
                        let text: String = text.chars().take(1000).collect();
                        self.class
                            .teacher
                            .inbox
                            .push(format!("{}: {text}", pupil.name));
                    }
                    LinkEvent::Message(ToTeacher::Bye) | LinkEvent::Closed(_) => {
                        closed.push(pupil.id)
                    }
                    LinkEvent::Frame(jpeg) => latest = Some(jpeg),
                }
            }
            if let Some(jpeg) = latest {
                pupil.last_frame = Some(Instant::now());
                upload(
                    ctx,
                    &mut pupil.texture,
                    &format!("pupil-{}", pupil.id),
                    &jpeg,
                );
                if show == Show::Student(pupil.id) {
                    forward = Some((pupil.id, Arc::new(jpeg)));
                }
            }
        }
        if let Some((from, jpeg)) = forward {
            for pupil in self.class.teacher.pupils.iter().filter(|p| p.id != from) {
                if !pupil.link.frame_pending() {
                    pupil.link.send_frame(jpeg.clone());
                }
            }
        }
        let inbox = &mut self.class.teacher.inbox;
        if inbox.len() > 200 {
            inbox.drain(..inbox.len() - 200);
        }
        if !closed.is_empty() {
            let teacher = &mut self.class.teacher;
            teacher.pupils.retain(|p| !closed.contains(&p.id));
            let live: Vec<_> = teacher.pupils.iter().map(|p| p.link.clone()).collect();
            *teacher.audience.lock().unwrap_or_else(|e| e.into_inner()) = live;
            if teacher.focus.is_some_and(|f| closed.contains(&f)) {
                teacher.focus = None;
                teacher.control = false;
            }
            if matches!(teacher.show, Show::Student(id) if closed.contains(&id)) {
                self.stop_show();
            }
        }
    }

    fn teacher_design(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        if self.class.teacher.server.is_none() {
            card(p).show(ui, |ui| {
                ui.set_max_width(520.);
                label(ui, self.tr("Sınıf oluştur", "Create a class"), 16., p.ink);
                label(
                    ui,
                    self.tr(
                        "Öğrenciler aynı ağdaysa sınıfı listede görür; yalnızca sınıf koduyla katılabilir.",
                        "Students on the same network see the class in a list and join with the class code.",
                    ),
                    12.,
                    p.muted,
                );
                ui.add_space(8.);
                ui.label(self.tr("Sınıf adı", "Class name"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.class.class_name)
                        .desired_width(f32::INFINITY)
                        .hint_text("10-A Bilişim"),
                );
                ui.label(self.tr("Sınıf kodu", "Class code"));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.settings.class.class_code)
                            .font(egui::TextStyle::Monospace)
                            .desired_width(160.)
                            .hint_text("ABCDE-FGHJK"),
                    );
                    if ui.button(self.tr("Yeni kod", "New code")).clicked() {
                        self.settings.class.class_code = classroom::generate_class_code();
                    }
                });
                label(
                    ui,
                    self.tr(
                        "Kod kaydedilir; sonraki derslerde öğrenciler aynı kodla katılır.",
                        "The code is remembered so students reuse it next lesson.",
                    ),
                    11.,
                    p.muted,
                );
                ui.add_space(8.);
                if button(ui, self.tr("Sınıfı başlat", "Start class"), p, true, 200.).clicked() {
                    self.start_class();
                }
            });
            label(ui, &self.class.status.clone(), 12., p.muted);
            return;
        }
        if let Some(id) = self.class.teacher.focus {
            self.focus_design(ui, ctx, p, id);
            return;
        }
        self.teacher_toolbar(ui, p);
        ui.add_space(8.);
        let count = self.class.teacher.pupils.len();
        if count == 0 {
            card(p).show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                label(
                    ui,
                    self.tr("Henüz öğrenci yok.", "No students yet."),
                    14.,
                    p.ink,
                );
                label(
                    ui,
                    self.tr(
                        "Öğrenci bilgisayarında Opendesk > Sınıf > Öğrenci'yi aç, sınıfı listeden seç ve kodu gir.",
                        "On each student computer open Opendesk > Class > Student, pick this class and enter the code.",
                    ),
                    12.,
                    p.muted,
                );
            });
        }
        let mut focus = None;
        let mut present = None;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let width = ui.available_width();
                let columns = ((width / 250.).floor() as usize).clamp(1, 8);
                let tile = (width - (columns as f32 - 1.) * 10.) / columns as f32;
                let english = self.settings.english;
                for row in self.class.teacher.pupils.chunks_mut(columns) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.;
                        for pupil in row {
                            ui.allocate_ui_with_layout(
                                Vec2::new(tile, tile * 0.625 + 48.),
                                egui::Layout::top_down(egui::Align::Min),
                                |ui| {
                                    ui.set_width(tile);
                                    egui::Frame::new()
                                        .fill(p.card)
                                        .stroke(egui::Stroke::new(
                                            if pupil.selected { 2.0_f32 } else { 1.0_f32 },
                                            if pupil.hand {
                                                Color32::from_rgb(230, 160, 40)
                                            } else if pupil.selected {
                                                p.accent
                                            } else {
                                                p.line
                                            },
                                        ))
                                        .corner_radius(10)
                                        .inner_margin(6)
                                        .show(ui, |ui| {
                                            let size = Vec2::new(tile - 14., (tile - 14.) * 0.625);
                                            let response = if let Some(texture) = &pupil.texture {
                                                fitted_image(
                                                    ui,
                                                    texture,
                                                    size,
                                                    egui::Sense::click(),
                                                )
                                            } else {
                                                let (rect, response) = ui.allocate_exact_size(
                                                    size,
                                                    egui::Sense::click(),
                                                );
                                                ui.painter().rect_filled(rect, 6, p.field);
                                                ui.painter().text(
                                                    rect.center(),
                                                    egui::Align2::CENTER_CENTER,
                                                    if english {
                                                        "Waiting for screen…"
                                                    } else {
                                                        "Ekran bekleniyor…"
                                                    },
                                                    egui::FontId::proportional(12.),
                                                    p.muted,
                                                );
                                                response
                                            };
                                            if response.clicked() {
                                                focus = Some(pupil.id);
                                            }
                                            ui.horizontal(|ui| {
                                                ui.checkbox(&mut pupil.selected, "");
                                                let mut name = pupil.name.clone();
                                                if pupil.hand {
                                                    name = format!("✋ {name}");
                                                }
                                                if pupil.locked {
                                                    name = format!("🔒 {name}");
                                                }
                                                let room = (ui.available_width() - 52.).max(20.);
                                                ui.allocate_ui(Vec2::new(room, 20.), |ui| {
                                                    ui.add(
                                                        egui::Label::new(
                                                            RichText::new(name)
                                                                .strong()
                                                                .color(p.ink),
                                                        )
                                                        .truncate(),
                                                    )
                                                    .on_hover_text(&pupil.computer);
                                                });
                                                ui.with_layout(
                                                    egui::Layout::right_to_left(
                                                        egui::Align::Center,
                                                    ),
                                                    |ui| {
                                                        if ui
                                                            .small_button("📺")
                                                            .on_hover_text(if english {
                                                                "Show this screen to everyone"
                                                            } else {
                                                                "Bu ekranı herkese göster"
                                                            })
                                                            .clicked()
                                                        {
                                                            present = Some(pupil.id);
                                                        }
                                                        let stale =
                                                            pupil.last_frame.is_none_or(|t| {
                                                                t.elapsed()
                                                                    > Duration::from_secs(10)
                                                            });
                                                        if stale {
                                                            ui.label(
                                                                RichText::new("●")
                                                                    .color(p.muted)
                                                                    .size(10.),
                                                            )
                                                            .on_hover_text(if english {
                                                                "No recent image"
                                                            } else {
                                                                "Görüntü gelmiyor"
                                                            });
                                                        }
                                                    },
                                                );
                                            });
                                        });
                                },
                            );
                        }
                    });
                    ui.add_space(10.);
                }
                if !self.class.teacher.inbox.is_empty() {
                    ui.separator();
                    label(
                        ui,
                        self.tr("Öğrenci mesajları", "Student messages"),
                        14.,
                        p.ink,
                    );
                    for message in self.class.teacher.inbox.iter().rev().take(20) {
                        ui.label(message);
                    }
                    if ui.small_button(self.tr("Temizle", "Clear")).clicked() {
                        self.class.teacher.inbox.clear();
                    }
                }
            });
        if let Some(id) = present {
            self.present_student(id);
        }
        if let Some(id) = focus {
            if let Some(pupil) = self.class.teacher.pupils.iter_mut().find(|p| p.id == id) {
                pupil.hand = false;
            }
            self.set_focus(Some(id));
        }
    }

    fn teacher_toolbar(&mut self, ui: &mut egui::Ui, p: Palette) {
        let address = match self.lan_ip {
            Some(ip) => format!("{ip}"),
            None => "?".into(),
        };
        card(p).inner_margin(14).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                label(ui, &self.settings.class.class_name.clone(), 18., p.ink);
                ui.separator();
                ui.label(self.tr("Kod:", "Code:"));
                ui.label(
                    RichText::new(&self.settings.class.class_code)
                        .monospace()
                        .size(20.)
                        .strong()
                        .color(p.accent),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("IP {address}"))
                        .monospace()
                        .color(p.muted),
                );
                ui.separator();
                let hands = self.class.teacher.pupils.iter().filter(|p| p.hand).count();
                ui.label(format!(
                    "{} {}{}",
                    self.class.teacher.pupils.len(),
                    self.tr("öğrenci", "students"),
                    if hands > 0 {
                        format!(" · ✋ {hands}")
                    } else {
                        String::new()
                    }
                ));
            });
            ui.add_space(6.);
            let selected = self
                .class
                .teacher
                .pupils
                .iter()
                .filter(|p| p.selected)
                .count();
            let scope = if selected > 0 {
                format!("{selected} {}", self.tr("seçili", "selected"))
            } else {
                self.tr("herkes", "everyone").to_owned()
            };
            ui.horizontal_wrapped(|ui| {
                match self.class.teacher.show {
                    Show::Nothing => {
                        if ui
                            .button(self.tr("🖥 Ekranımı yansıt", "🖥 Present my screen"))
                            .clicked()
                        {
                            self.present_mine();
                        }
                    }
                    _ => {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(
                                        self.tr("■ Yansıtmayı durdur", "■ Stop presenting"),
                                    )
                                    .color(Color32::WHITE),
                                )
                                .fill(p.red),
                            )
                            .clicked()
                        {
                            self.stop_show();
                        }
                    }
                }
                if ui
                    .button(format!("🔒 {} ({scope})", self.tr("Kilitle", "Lock")))
                    .clicked()
                {
                    self.lock(true);
                }
                if ui
                    .button(format!("🔓 {} ({scope})", self.tr("Kilidi aç", "Unlock")))
                    .clicked()
                {
                    self.lock(false);
                }
                if ui.button(self.tr("Tümünü seç", "Select all")).clicked() {
                    let all = self.class.teacher.pupils.iter().all(|p| p.selected);
                    for pupil in &mut self.class.teacher.pupils {
                        pupil.selected = !all;
                    }
                }
                if ui
                    .add(
                        egui::Button::new(self.tr("Sınıfı kapat", "End class"))
                            .stroke(egui::Stroke::new(1.0_f32, p.red)),
                    )
                    .clicked()
                {
                    self.end_class();
                }
            });
            ui.horizontal(|ui| {
                let send = ui.add(
                    egui::TextEdit::singleline(&mut self.class.teacher.message)
                        .desired_width((ui.available_width() - 140.).max(120.))
                        .hint_text(if self.settings.english {
                            "Message to students…"
                        } else {
                            "Öğrencilere mesaj…"
                        }),
                );
                let enter = send.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (ui.button(format!("✉ {scope}")).clicked() || enter)
                    && !self.class.teacher.message.trim().is_empty()
                {
                    let text: String = self
                        .class
                        .teacher
                        .message
                        .trim()
                        .chars()
                        .take(1000)
                        .collect();
                    self.broadcast(ToStudent::Message(text));
                    self.class.teacher.message.clear();
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.class.teacher.url)
                        .desired_width((ui.available_width() - 140.).max(120.))
                        .hint_text("https://…"),
                );
                if ui
                    .button(format!("🌐 {}", self.tr("Siteyi aç", "Open site")))
                    .clicked()
                {
                    let url = self.class.teacher.url.trim().to_owned();
                    if url.starts_with("https://") || url.starts_with("http://") {
                        self.broadcast(ToStudent::OpenUrl(url));
                    } else {
                        self.class_status(
                            "Adres https:// ile başlamalı.",
                            "The address must start with https://.",
                        );
                    }
                }
            });
            if !self.class.status.is_empty() {
                label(ui, &self.class.status.clone(), 11., p.muted);
            }
        });
    }

    fn focus_design(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette, id: u64) {
        let Some(index) = self.class.teacher.pupils.iter().position(|p| p.id == id) else {
            self.set_focus(None);
            return;
        };
        let name = self.class.teacher.pupils[index].name.clone();
        let mut back = false;
        let mut toggle_control = None;
        let mut present = false;
        ui.horizontal_wrapped(|ui| {
            if ui.button(self.tr("← Sınıf", "← Class")).clicked() {
                back = true;
            }
            label(ui, &name, 16., p.ink);
            ui.separator();
            let control = self.class.teacher.control;
            if ui
                .selectable_label(control, self.tr("🖱 Kontrol et", "🖱 Take control"))
                .clicked()
            {
                toggle_control = Some(!control);
            }
            if ui
                .button(self.tr("📺 Herkese göster", "📺 Show to everyone"))
                .clicked()
            {
                present = true;
            }
        });
        if back {
            self.set_focus(None);
            return;
        }
        if let Some(enabled) = toggle_control {
            self.set_control(enabled);
        }
        if present {
            self.present_student(id);
        }
        let size = ui.available_size();
        let control = self.class.teacher.control;
        let pupil = &self.class.teacher.pupils[index];
        let Some(texture) = pupil.texture.clone() else {
            ui.centered_and_justified(|ui| {
                ui.spinner();
            });
            return;
        };
        let response = fitted_image(ui, &texture, size, egui::Sense::click_and_drag());
        if !control {
            return;
        }
        let link = pupil.link.clone();
        if response.hovered() {
            response.request_focus();
        }
        let focused = response.has_focus() && ctx.input(|i| i.focused);
        if self.class.teacher.controlling && !focused {
            link.send(ToStudent::Input(opendesk::network::InputEvent::ReleaseAll));
        }
        self.class.teacher.controlling = focused;
        if ctx.input(|i| i.pointer.delta() != Vec2::ZERO)
            && let Some(pos) = response.hover_pos()
        {
            let rel = (pos - response.rect.min) / response.rect.size();
            link.send(ToStudent::Input(opendesk::network::InputEvent::Move {
                x: rel.x.clamp(0., 1.),
                y: rel.y.clamp(0., 1.),
            }));
        }
        let (events, modifiers) = if focused {
            ctx.input(|i| (i.events.clone(), i.modifiers))
        } else {
            (Vec::new(), egui::Modifiers::NONE)
        };
        for input in translate_remote_events(
            &events,
            self.class.teacher.modifiers,
            modifiers,
            response.hovered(),
        ) {
            link.send(ToStudent::Input(input));
        }
        self.class.teacher.modifiers = modifiers;
    }

    // ---------------------------------------------------------------- student

    pub(crate) fn join_class(&mut self) {
        let prefs = &self.settings.class;
        if prefs.student_name.trim().is_empty() {
            self.class_status("Adını yaz.", "Enter your name.");
            return;
        }
        if classroom::normalize_code(&prefs.last_code).is_none() {
            self.class_status(
                "Sınıf kodu 10 karakter olmalı (ör. ABCDE-FGHJK).",
                "The class code has 10 characters (e.g. ABCDE-FGHJK).",
            );
            return;
        }
        if prefs.last_address.trim().is_empty() {
            self.class_status(
                "Listeden bir sınıf seç veya öğretmenin IP adresini yaz.",
                "Pick a class from the list or type the teacher's IP address.",
            );
            return;
        }
        let address = prefs.last_address.clone();
        let code = prefs.last_code.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(classroom::join(&address, &code));
        });
        self.class.student.joining = Some(rx);
        self.class.student.wants = true;
        self.class.student.retry_at = None;
        self.class_status("Sınıfa bağlanılıyor…", "Joining the class…");
        self.persist_class();
    }
    fn leave_class(&mut self) {
        let student = &mut self.class.student;
        student.wants = false;
        student.retry_at = None;
        student.joining = None;
        student.joined = None;
        student.show = None;
        student.show_texture = None;
        student.lock = None;
        student.hand = false;
        self.class_status("Sınıftan ayrıldın.", "You left the class.");
    }
    fn student_connected(&mut self, link: TeacherLink) {
        let link = Arc::new(link);
        link.send(ToTeacher::Hello {
            name: self.settings.class.student_name.trim().to_owned(),
            computer: format!("{} · {}", computer_name(), user_name()),
        });
        let capture = Arc::new(Mutex::new(THUMBNAIL));
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (link, capture, stop) = (link.clone(), capture.clone(), stop.clone());
            std::thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    let (fps, quality, width) = *capture.lock().unwrap_or_else(|e| e.into_inner());
                    if fps == 0 || link.frame_pending() {
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    let started = Instant::now();
                    if let Ok(jpeg) = platform::capture_display_scaled(0, quality, width) {
                        link.send_frame(Arc::new(jpeg));
                    }
                    std::thread::sleep(
                        Duration::from_millis(1000 / u64::from(fps.max(1)))
                            .saturating_sub(started.elapsed()),
                    );
                }
            });
        }
        self.class.student.joined = Some(Joined {
            link,
            class: String::new(),
            capture,
            stop,
            input: None,
        });
        self.class_status("Sınıfa katıldın.", "You joined the class.");
    }
    fn poll_student(&mut self, ctx: &egui::Context) {
        if let Some(result) = self
            .class
            .student
            .joining
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.class.student.joining = None;
            match result {
                Ok(link) => self.student_connected(link),
                Err(e) => {
                    self.class.status = e;
                    self.class.student.retry_at = Some(Instant::now() + RETRY_EVERY);
                }
            }
        }
        let student = &mut self.class.student;
        if student.wants
            && student.joined.is_none()
            && student.joining.is_none()
            && student.retry_at.is_some_and(|t| Instant::now() >= t)
        {
            self.join_class();
        }
        let mut closed = false;
        let mut latest = None;
        let mut status = None;
        let student = &mut self.class.student;
        if let Some(joined) = &mut student.joined {
            for event in joined.link.poll_events() {
                match event {
                    LinkEvent::Message(ToStudent::Welcome { class }) => {
                        joined.class = class.chars().take(64).collect()
                    }
                    LinkEvent::Message(ToStudent::Capture {
                        fps,
                        quality,
                        max_width,
                    }) => {
                        *joined.capture.lock().unwrap_or_else(|e| e.into_inner()) = (
                            fps.min(30),
                            quality.clamp(20, 95),
                            max_width.clamp(160, 3840),
                        );
                    }
                    LinkEvent::Message(ToStudent::ShowStart { title }) => {
                        student.show = Some(title.chars().take(64).collect())
                    }
                    LinkEvent::Message(ToStudent::ShowEnd) => {
                        student.show = None;
                        student.show_texture = None;
                    }
                    LinkEvent::Message(ToStudent::Lock { locked, message }) => {
                        student.lock = locked.then(|| message.chars().take(300).collect());
                    }
                    LinkEvent::Message(ToStudent::Message(text)) => {
                        let text: String = text.chars().take(1000).collect();
                        student.messages.push(text.clone());
                        student.popup = Some(text);
                    }
                    LinkEvent::Message(ToStudent::OpenUrl(url)) => {
                        if let Err(e) = open_url(&url) {
                            status = Some(e);
                        }
                    }
                    LinkEvent::Message(ToStudent::Control(enabled)) => {
                        joined.input = None;
                        if enabled {
                            match platform::InputController::new(0) {
                                Ok(c) => joined.input = Some(c),
                                Err(e) => status = Some(format!("Control unavailable: {e}")),
                            }
                        }
                    }
                    LinkEvent::Message(ToStudent::Input(input)) => {
                        if let Some(controller) = &mut joined.input
                            && classroom::check_input(&input).is_ok()
                        {
                            let _ = controller.apply(input);
                        }
                    }
                    LinkEvent::Message(ToStudent::Bye) | LinkEvent::Closed(_) => closed = true,
                    LinkEvent::Frame(jpeg) => latest = Some(jpeg),
                }
            }
        }
        if let Some(text) = status {
            self.class.status = text;
        }
        let student = &mut self.class.student;
        if let Some(jpeg) = latest
            && student.show.is_some()
        {
            upload(ctx, &mut student.show_texture, "teacher-show", &jpeg);
        }
        if student.messages.len() > 100 {
            student.messages.remove(0);
        }
        if closed {
            // Never leave a student locked or presented to once the teacher is gone.
            student.joined = None;
            student.show = None;
            student.show_texture = None;
            student.lock = None;
            student.hand = false;
            if student.wants {
                student.retry_at = Some(Instant::now() + RETRY_EVERY);
                self.class_status(
                    "Öğretmenle bağlantı kesildi; yeniden bağlanılıyor…",
                    "Lost the teacher; reconnecting…",
                );
            }
        }
        let student = &mut self.class.student;
        let takeover = student.show.is_some() || student.lock.is_some();
        if takeover != student.fullscreen_applied {
            student.fullscreen_applied = takeover;
            if takeover {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::AlwaysOnTop,
                ));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
                    egui::WindowLevel::Normal,
                ));
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
            }
        }
    }

    /// Full-window view while the teacher presents or locks the screen. Returns true
    /// when it took over the window.
    pub(crate) fn student_takeover(&mut self, ctx: &egui::Context) -> bool {
        let student = &self.class.student;
        if student.lock.is_none() && student.show.is_none() {
            return false;
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(Color32::from_rgb(12, 14, 18)))
            .show(ctx, |ui| {
                let student = &self.class.student;
                if let Some(message) = &student.lock {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.3);
                        ui.label(RichText::new("🔒").size(72.).color(Color32::WHITE));
                        ui.label(
                            RichText::new(self.tr(
                                "Ekranın öğretmen tarafından kilitlendi",
                                "Your screen was locked by the teacher",
                            ))
                            .size(28.)
                            .color(Color32::WHITE),
                        );
                        if !message.is_empty() {
                            ui.add_space(12.);
                            ui.label(
                                RichText::new(message)
                                    .size(18.)
                                    .color(Color32::from_gray(190)),
                            );
                        }
                    });
                } else if let Some(texture) = &student.show_texture {
                    let size = ui.available_size();
                    fitted_image(ui, texture, size, egui::Sense::hover());
                } else {
                    ui.centered_and_justified(|ui| {
                        ui.label(
                            RichText::new(self.tr(
                                "Öğretmenin ekranı bekleniyor…",
                                "Waiting for the teacher's screen…",
                            ))
                            .size(20.)
                            .color(Color32::WHITE),
                        );
                    });
                }
            });
        true
    }

    fn student_design(&mut self, ui: &mut egui::Ui, p: Palette) {
        let student = &mut self.class.student;
        if student.joined.is_none()
            && student.discovery.is_none()
            && student.discovery_retry.is_none_or(|t| Instant::now() >= t)
        {
            // The beacon port is busy when another Opendesk runs on this computer;
            // joining by IP still works, so only retry occasionally.
            match classroom::discover() {
                Ok(d) => student.discovery = Some(d),
                Err(_) => student.discovery_retry = Some(Instant::now() + Duration::from_secs(10)),
            }
        }
        if let Some(d) = &self.class.student.discovery {
            for found in d.found.try_iter() {
                let list = &mut self.class.student.found;
                match list.iter_mut().find(|f| f.address == found.address) {
                    Some(existing) => *existing = found,
                    None => list.push(found),
                }
            }
        }
        if let Some(joined) = &self.class.student.joined {
            let class = joined.class.clone();
            let controlled = joined.input.is_some();
            card(p).show(ui, |ui| {
                ui.set_max_width(560_f32.min(ui.available_width()));
                label(
                    ui,
                    &format!("{}: {class}", self.tr("Sınıf", "Class")),
                    16.,
                    p.ink,
                );
                ui.label(
                    RichText::new(self.tr(
                        "● Öğretmen ekranını görebilir.",
                        "● The teacher can see your screen.",
                    ))
                    .color(p.red),
                );
                if controlled {
                    ui.label(
                        RichText::new(self.tr(
                            "● Öğretmen bilgisayarını kontrol ediyor.",
                            "● The teacher is controlling your computer.",
                        ))
                        .color(p.red),
                    );
                }
                ui.add_space(8.);
                let hand = self.class.student.hand;
                if button(
                    ui,
                    if hand {
                        self.tr("✋ Elini indir", "✋ Lower hand")
                    } else {
                        self.tr("✋ Söz iste / yardım", "✋ Raise hand / ask for help")
                    },
                    p,
                    !hand,
                    240.,
                )
                .clicked()
                    && let Some(joined) = &self.class.student.joined
                    && joined.link.send(ToTeacher::Hand(!hand))
                {
                    self.class.student.hand = !hand;
                }
                ui.add_space(6.);
                ui.horizontal(|ui| {
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut self.class.student.message)
                            .desired_width((ui.available_width() - 90.).max(120.))
                            .hint_text(if self.settings.english {
                                "Message to the teacher…"
                            } else {
                                "Öğretmene mesaj…"
                            }),
                    );
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (ui.button(self.tr("Gönder", "Send")).clicked() || enter)
                        && !self.class.student.message.trim().is_empty()
                        && let Some(joined) = &self.class.student.joined
                    {
                        let text: String = self
                            .class
                            .student
                            .message
                            .trim()
                            .chars()
                            .take(1000)
                            .collect();
                        if joined.link.send(ToTeacher::Message(text)) {
                            self.class.student.message.clear();
                        }
                    }
                });
                if !self.class.student.messages.is_empty() {
                    ui.separator();
                    label(ui, self.tr("Öğretmenden", "From the teacher"), 13., p.muted);
                    for message in self.class.student.messages.iter().rev().take(10) {
                        ui.label(message);
                    }
                }
                ui.add_space(8.);
                if ui
                    .button(self.tr("Sınıftan ayrıl", "Leave class"))
                    .clicked()
                {
                    self.leave_class();
                }
            });
            label(ui, &self.class.status.clone(), 12., p.muted);
            return;
        }
        card(p).show(ui, |ui| {
            ui.set_max_width(560_f32.min(ui.available_width()));
            label(ui, self.tr("Sınıfa katıl", "Join a class"), 16., p.ink);
            ui.add_space(6.);
            ui.label(self.tr("Adın soyadın", "Your name"));
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.class.student_name)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.);
            label(
                ui,
                self.tr("Ağdaki sınıflar", "Classes on this network"),
                13.,
                p.muted,
            );
            if self.class.student.found.is_empty() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr("Sınıf aranıyor…", "Looking for classes…"));
                });
            }
            let mut chosen = None;
            for found in &self.class.student.found {
                let selected = self.settings.class.last_address == found.address.to_string();
                if ui
                    .selectable_label(
                        selected,
                        format!("🏫 {}  ({})", found.name, found.address.ip()),
                    )
                    .clicked()
                {
                    chosen = Some(found.address.to_string());
                }
            }
            if let Some(address) = chosen {
                self.settings.class.last_address = address;
            }
            ui.label(self.tr("veya öğretmenin IP adresi", "or the teacher's IP address"));
            ui.add(
                egui::TextEdit::singleline(&mut self.settings.class.last_address)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .hint_text("192.168.1.10"),
            );
            ui.label(self.tr("Sınıf kodu", "Class code"));
            let code = ui.add(
                egui::TextEdit::singleline(&mut self.settings.class.last_code)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .hint_text("ABCDE-FGHJK"),
            );
            let enter = code.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let autojoin = self
                .tr(
                    "Opendesk açılınca bu sınıfa otomatik katıl",
                    "Join this class automatically when Opendesk starts",
                )
                .to_owned();
            ui.checkbox(&mut self.settings.class.autojoin, autojoin);
            ui.add_space(6.);
            let busy = self.class.student.joining.is_some();
            if busy {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr("Bağlanılıyor…", "Connecting…"));
                    if ui.button(self.tr("Vazgeç", "Cancel")).clicked() {
                        self.leave_class();
                    }
                });
            } else if self.class.student.wants {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(self.tr("Yeniden denenecek…", "Retrying soon…"));
                    if ui.button(self.tr("Vazgeç", "Cancel")).clicked() {
                        self.leave_class();
                    }
                });
            } else if button(ui, self.tr("Katıl", "Join"), p, true, 200.).clicked() || enter {
                self.join_class();
            }
        });
        label(ui, &self.class.status.clone(), 12., p.muted);
    }

    // ---------------------------------------------------------------- shared

    pub(crate) fn poll_class(&mut self, ctx: &egui::Context) {
        if !self.class.autojoin_checked {
            self.class.autojoin_checked = true;
            if self.settings.role == Some(Role::Student) && self.settings.class.autojoin {
                self.join_class();
            }
        }
        self.poll_teacher(ctx);
        self.poll_student(ctx);
        if self.class.teacher.server.is_some() || self.class.student.joined.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    pub(crate) fn class_design(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        let teaching = self.class.teacher.server.is_some();
        let learning = self.class.student.joined.is_some() || self.class.student.wants;
        if !teaching && !learning {
            ui.horizontal(|ui| {
                ui.heading(self.tr("Sınıf", "Classroom"));
                ui.add_space(16.);
                let mut role = self.settings.role.unwrap_or(Role::Personal);
                let before = role;
                ui.selectable_value(&mut role, Role::Teacher, self.tr("Öğretmen", "Teacher"));
                ui.selectable_value(&mut role, Role::Student, self.tr("Öğrenci", "Student"));
                if role != before {
                    self.settings.role = Some(role);
                    self.persist_class();
                }
            });
            ui.add_space(10.);
        }
        if let Some(text) = self.class.student.popup.clone() {
            egui::Modal::new(egui::Id::new("teacher-message")).show(ctx, |ui| {
                ui.set_width(380.);
                ui.heading(self.tr("Öğretmenden mesaj", "Message from the teacher"));
                ui.add_space(8.);
                ui.label(RichText::new(text).size(16.));
                ui.add_space(8.);
                if ui.button(self.tr("Tamam", "OK")).clicked() {
                    self.class.student.popup = None;
                }
            });
        }
        let teacher_view = teaching || (!learning && self.settings.role == Some(Role::Teacher));
        if teacher_view {
            self.teacher_design(ui, ctx, p);
        } else if learning || self.settings.role == Some(Role::Student) {
            egui::ScrollArea::vertical().show(ui, |ui| self.student_design(ui, p));
        } else {
            card(p).show(ui, |ui| {
                ui.set_max_width(560_f32.min(ui.available_width()));
                label(
                    ui,
                    self.tr(
                        "Bu bilgisayar sınıfta nasıl kullanılacak? Yukarıdan Öğretmen veya Öğrenci seç.",
                        "How is this computer used in class? Choose Teacher or Student above.",
                    ),
                    14.,
                    p.ink,
                );
            });
        }
    }
}

impl Desk {
    /// First launch without a role chosen at installation: ask once.
    pub(crate) fn role_design(&mut self, ctx: &egui::Context, p: Palette) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(30))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space((ui.available_height() * 0.12).min(80.));
                    ui.heading(self.tr("Opendesk'e hoş geldin", "Welcome to Opendesk"));
                    label(
                        ui,
                        self.tr(
                            "Bu bilgisayar nasıl kullanılacak? Sonra Ayarlar'dan değiştirebilirsin.",
                            "How will this computer be used? You can change it later in Settings.",
                        ),
                        14.,
                        p.muted,
                    );
                    ui.add_space(20.);
                    let choices = [
                        (Role::Personal, "🖥", "Bireysel", "Personal", "Uzak masaüstü: bir bilgisayara bağlan veya ekranını paylaş.", "Remote desktop: connect to a computer or share yours."),
                        (Role::Teacher, "🎓", "Öğretmen", "Teacher", "Sınıf oluştur, öğrenci ekranlarını izle, ekranını yansıt.", "Create a class, watch student screens, present yours."),
                        (Role::Student, "🎒", "Öğrenci", "Student", "Öğretmenin sınıfına katıl.", "Join the teacher's class."),
                    ];
                    let width = ui.available_width().min(760.);
                    let narrow = width < 600.;
                    let tile = if narrow { width } else { (width - 24.) / 3. };
                    let mut picked = None;
                    let mut draw = |ui: &mut egui::Ui| {
                        for (role, icon, tr, en, dtr, den) in choices {
                            let english = self.settings.english;
                            let response = card(p)
                                .show(ui, |ui| {
                                    ui.set_width(tile - 46.);
                                    ui.set_min_height(if narrow { 0. } else { 150. });
                                    ui.vertical_centered(|ui| {
                                        ui.label(RichText::new(icon).size(36.));
                                        label(ui, if english { en } else { tr }, 18., p.ink);
                                        label(ui, if english { den } else { dtr }, 12., p.muted);
                                    });
                                })
                                .response
                                .interact(egui::Sense::click());
                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.clicked() {
                                picked = Some(role);
                            }
                        }
                    };
                    if narrow {
                        ui.vertical(|ui| draw(ui));
                    } else {
                        ui.allocate_ui(Vec2::new(width, 220.), |ui| {
                            ui.horizontal_top(|ui| draw(ui));
                        });
                    }
                    if let Some(role) = picked {
                        self.settings.role = Some(role);
                        self.ui_page = match role {
                            Role::Personal => crate::design::Page::Home,
                            _ => crate::design::Page::Classroom,
                        };
                        self.persist_class();
                    }
                });
            });
    }
}
