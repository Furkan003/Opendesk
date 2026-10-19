//! Native rendering of the supplied design, driven entirely by real session state.
use crate::{Command, Desk, settings};
use eframe::egui::{self, Color32, CornerRadius, Rect, RichText, Stroke, Vec2};
pub fn icon_rgba() -> egui::IconData {
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    let inside = |x: f32, y: f32, left: f32, top: f32, w: f32, r: f32| {
        let dx = (left + r - x).max(0.).max(x - (left + w - r));
        let dy = (top + r - y).max(0.).max(y - (top + w - r));
        x >= left && x <= left + w && y >= top && y <= top + w && dx * dx + dy * dy <= r * r
    };
    for y in 0..64 {
        for x in 0..64 {
            let (x, y) = (x as f32 + 0.5, y as f32 + 0.5);
            let blue = [50, 105, 187, 255];
            let color = if !inside(x, y, 0., 0., 64., 14.) {
                [0, 0, 0, 0]
            } else if inside(x, y, 27., 27., 24., 6.) {
                [255, 255, 255, 255]
            } else if inside(x, y, 23.5, 23.5, 31., 9.) {
                blue
            } else if inside(x, y, 13., 13., 24., 6.) && !inside(x, y, 16.5, 16.5, 17., 3.) {
                [255, 255, 255, 255]
            } else {
                blue
            };
            rgba.extend(color);
        }
    }
    egui::IconData {
        rgba,
        width: 64,
        height: 64,
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Home,
    Devices,
    Classroom,
    Settings,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Chat,
    Files,
    Clipboard,
}
#[derive(Clone, Copy)]
pub(crate) struct Palette {
    pub(crate) bg: Color32,
    pub(crate) side: Color32,
    pub(crate) card: Color32,
    pub(crate) line: Color32,
    pub(crate) ink: Color32,
    pub(crate) muted: Color32,
    pub(crate) accent: Color32,
    pub(crate) soft: Color32,
    pub(crate) green: Color32,
    pub(crate) red: Color32,
    pub(crate) field: Color32,
}
impl Palette {
    fn new(dark: bool) -> Self {
        if dark {
            Self {
                bg: Color32::from_rgb(20, 23, 29),
                side: Color32::from_rgb(15, 18, 24),
                card: Color32::from_rgb(27, 31, 38),
                line: Color32::from_rgb(46, 51, 60),
                ink: Color32::from_rgb(233, 236, 242),
                muted: Color32::from_rgb(153, 161, 176),
                accent: Color32::from_rgb(106, 157, 235),
                soft: Color32::from_rgb(32, 47, 73),
                green: Color32::from_rgb(90, 183, 136),
                red: Color32::from_rgb(213, 93, 84),
                field: Color32::from_rgb(20, 23, 29),
            }
        } else {
            Self {
                bg: Color32::from_rgb(246, 247, 250),
                side: Color32::from_rgb(238, 241, 246),
                card: Color32::from_rgb(253, 254, 255),
                line: Color32::from_rgb(219, 224, 233),
                ink: Color32::from_rgb(25, 32, 45),
                muted: Color32::from_rgb(91, 101, 119),
                accent: Color32::from_rgb(50, 105, 187),
                soft: Color32::from_rgb(228, 237, 251),
                green: Color32::from_rgb(42, 131, 83),
                red: Color32::from_rgb(190, 65, 52),
                field: Color32::from_rgb(244, 246, 249),
            }
        }
    }
}
pub fn configure_fonts(ctx: &egui::Context) {
    let mut f = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "Plex Sans",
            include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").as_slice(),
        ),
        (
            "Plex Mono",
            include_bytes!("../assets/fonts/IBMPlexMono-Regular.ttf").as_slice(),
        ),
        (
            "Plex Semibold",
            include_bytes!("../assets/fonts/IBMPlexSans-SemiBold.ttf").as_slice(),
        ),
    ] {
        f.font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    f.families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "Plex Sans".into());
    f.families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, "Plex Mono".into());
    // Symbols such as ● ▤ ✋ 🔒 come from egui's bundled fonts; headings need the
    // same fallbacks or those glyphs render as empty boxes.
    let fallbacks: Vec<String> = ["NotoEmoji-Regular", "emoji-icon-font", "Hack"]
        .into_iter()
        .filter(|name| f.font_data.contains_key(*name))
        .map(String::from)
        .collect();
    let proportional = f
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    for name in &fallbacks {
        if !proportional.contains(name) {
            proportional.push(name.clone());
        }
    }
    let mut semibold = vec!["Plex Semibold".to_owned(), "Plex Sans".to_owned()];
    semibold.extend(fallbacks);
    f.families
        .insert(egui::FontFamily::Name("Plex Semibold".into()), semibold);
    ctx.set_fonts(f);
}
pub(crate) fn card(p: Palette) -> egui::Frame {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0_f32, p.line))
        .corner_radius(14)
        .inner_margin(22)
}
pub(crate) fn label(ui: &mut egui::Ui, text: &str, size: f32, color: Color32) {
    let mut text = RichText::new(text).size(size).color(color);
    if size >= 16.0 {
        text = text.family(egui::FontFamily::Name("Plex Semibold".into()));
    }
    ui.label(text);
}
pub(crate) fn button(
    ui: &mut egui::Ui,
    text: &str,
    p: Palette,
    primary: bool,
    width: f32,
) -> egui::Response {
    ui.add_sized(
        [width, 40.],
        egui::Button::new(RichText::new(text).size(14.).color(if primary {
            Color32::WHITE
        } else {
            p.ink
        }))
        .fill(if primary { p.accent } else { p.card })
        .stroke(Stroke::new(
            1.0_f32,
            if primary { p.accent } else { p.line },
        ))
        .corner_radius(9),
    )
}
fn logo(ui: &mut egui::Ui, size: f32, variant: u8, p: Palette) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let s = size / 28.;
    let painter = ui.painter();
    match variant {
        1 => {
            let points: Vec<_> = (0..=32)
                .map(|i| {
                    let a = (135. + i as f32 * 270. / 32.).to_radians();
                    r.center() + Vec2::new(a.cos(), a.sin()) * 11.5 * s
                })
                .collect();
            painter.add(egui::Shape::line(points, Stroke::new(5. * s, p.accent)));
            painter.circle_filled(r.center(), 4. * s, p.accent);
        }
        2 => {
            let d = Vec2::new(0.819152, -0.573576) * 8. * s;
            painter.line_segment(
                [r.center() - d, r.center() + d],
                Stroke::new(3. * s, p.accent),
            );
            painter.circle_filled(r.center() - d, 5. * s, p.accent);
            painter.circle_filled(r.center() + d, 5. * s, p.card);
            painter.circle_stroke(r.center() + d, 4. * s, Stroke::new(3. * s, p.accent));
        }
        _ => {
            painter.rect_stroke(
                Rect::from_min_size(r.min + Vec2::splat(1.25 * s), Vec2::splat(15.5 * s)),
                CornerRadius::same((4. * s) as u8),
                Stroke::new(2.5 * s, p.accent),
                egui::StrokeKind::Middle,
            );
            let front = Rect::from_min_size(r.min + Vec2::splat(10. * s), Vec2::splat(18. * s));
            painter.rect_filled(
                front.expand(2.5 * s),
                CornerRadius::same((7.5 * s) as u8),
                p.side,
            );
            painter.rect_filled(front, CornerRadius::same((5. * s) as u8), p.accent);
        }
    }
}
fn monitor(ui: &mut egui::Ui, size: f32, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let s = Stroke::new(1.6_f32, color);
    let p = ui.painter();
    p.rect_stroke(
        Rect::from_min_max(r.min + Vec2::new(2., 2.), r.max - Vec2::new(2., 8.)),
        3,
        s,
        egui::StrokeKind::Inside,
    );
    p.line_segment(
        [
            r.center() + Vec2::new(0., size / 2. - 8.),
            r.center() + Vec2::new(0., size / 2. - 3.),
        ],
        s,
    );
    p.line_segment(
        [r.min + Vec2::new(6., size - 3.), r.max - Vec2::new(6., 3.)],
        s,
    );
}
impl Desk {
    pub(crate) fn tr<'a>(&self, tr: &'a str, en: &'a str) -> &'a str {
        if self.settings.english { en } else { tr }
    }
    fn status_text(&self) -> &str {
        if self.status == "Ready. Both computers must run Opendesk." {
            self.tr(
                "Hazır. Her iki bilgisayarda da Opendesk çalışmalı.",
                "Ready. Both computers must run Opendesk.",
            )
        } else {
            &self.status
        }
    }
    fn persist_design(&mut self) {
        if let Err(e) = self.settings.save() {
            self.status = e;
        }
    }
    fn apply_design(&self, ctx: &egui::Context, p: Palette) {
        let mut v = if self.settings.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        v.panel_fill = p.bg;
        v.window_fill = p.card;
        v.extreme_bg_color = p.field;
        v.faint_bg_color = p.field;
        v.override_text_color = Some(p.ink);
        v.selection.bg_fill = p.soft;
        v.selection.stroke = Stroke::new(1.0_f32, p.accent);
        v.widgets.inactive.bg_fill = p.card;
        v.widgets.inactive.weak_bg_fill = p.card;
        v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.line);
        v.widgets.hovered.bg_fill = p.soft;
        v.widgets.hovered.weak_bg_fill = p.soft;
        v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
        v.widgets.active.bg_fill = p.soft;
        v.widgets.active.weak_bg_fill = p.soft;
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.noninteractive,
        ] {
            w.corner_radius = CornerRadius::same(8);
        }
        ctx.set_visuals(v);
        ctx.style_mut(|s| {
            s.spacing.item_spacing = Vec2::new(8., 10.);
            s.spacing.button_padding = Vec2::new(12., 8.);
            s.spacing.interact_size = Vec2::new(36., 32.);
            s.text_styles
                .insert(egui::TextStyle::Body, egui::FontId::proportional(13.));
            s.text_styles
                .insert(egui::TextStyle::Button, egui::FontId::proportional(13.));
            s.text_styles
                .insert(egui::TextStyle::Heading, egui::FontId::proportional(24.));
        });
    }
    pub(crate) fn design_ui(&mut self, ctx: &egui::Context) {
        let p = Palette::new(self.settings.dark);
        self.apply_design(ctx, p);
        if self.fullscreen && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.fullscreen = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
        }
        if self.student_takeover(ctx) {
            return;
        }
        if self.settings.role.is_none() && self.ui_capture.is_none() {
            self.role_design(ctx, p);
            return;
        }
        if self.connected {
            self.session_design(ctx, p);
        } else if self.ui_page == Page::Classroom
            && self.connecting.is_none()
            && (self.hosting || self.connection.is_none())
            && self.retry_at.is_none()
        {
            // The class view sizes itself to the window, so it is not in a scroll area.
            self.navigation(ctx, p);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(p.bg)
                        .inner_margin(egui::Margin::symmetric(24, 20)),
                )
                .show(ctx, |ui| self.class_design(ui, ctx, p));
        } else {
            self.navigation(ctx, p);
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(p.bg)
                        .inner_margin(egui::Margin::symmetric(36, 30)),
                )
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if self.connecting.is_some()
                            || (!self.hosting && self.connection.is_some())
                            || self.retry_at.is_some()
                        {
                            self.connecting_design(ui, p);
                        } else {
                            match self.ui_page {
                                Page::Home | Page::Classroom => self.home_design(ui, ctx, p),
                                Page::Devices => self.devices_design(ui, p),
                                Page::Settings => self.settings_design(ui, ctx, p),
                            }
                        }
                    });
                });
        }
        self.request_design(ctx, p);
    }
    fn navigation(&mut self, ctx: &egui::Context, p: Palette) {
        // Narrow windows (small laptops, split screens) get an icon-only sidebar.
        let compact = ctx.screen_rect().width() < 900.;
        egui::SidePanel::left("design-navigation")
            .exact_width(if compact { 64. } else { 216. })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.side)
                    .inner_margin(egui::Margin::symmetric(12, 18)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(if compact { 6. } else { 8. });
                    logo(ui, 28., self.settings.logo, p);
                    if !compact {
                        label(ui, "opendesk", 18., p.ink);
                    }
                });
                ui.add_space(22.);
                ui.spacing_mut().item_spacing.y = 2.;
                for (page, icon, tr, en) in [
                    (Page::Home, "↗", "Bağlan", "Connect"),
                    (Page::Devices, "▤", "Cihazlar", "Devices"),
                    (Page::Classroom, "🎓", "Sınıf", "Classroom"),
                    (Page::Settings, "⚙", "Ayarlar", "Settings"),
                ] {
                    let selected = self.ui_page == page;
                    let text = if compact {
                        icon.to_owned()
                    } else {
                        format!("{icon}    {}", self.tr(tr, en))
                    };
                    let (rect, response) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), 38.),
                        egui::Sense::click(),
                    );
                    ui.painter().rect(
                        rect,
                        8,
                        if selected {
                            p.card
                        } else if response.hovered() {
                            p.soft
                        } else {
                            p.side
                        },
                        Stroke::new(1.0_f32, if selected { p.line } else { p.side }),
                        egui::StrokeKind::Inside,
                    );
                    ui.painter().text(
                        rect.min + Vec2::new(12., 19.),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(14.),
                        if selected { p.ink } else { p.muted },
                    );
                    let response = if compact {
                        response.on_hover_text(self.tr(tr, en))
                    } else {
                        response
                    };
                    if response.clicked() {
                        self.ui_page = page;
                    }
                }
                ui.spacing_mut().item_spacing.y = 10.;
                let (online, state) = if self.class.teacher_active() {
                    (true, self.tr("Sınıf açık", "Class open"))
                } else if self.class.student_active() {
                    (true, self.tr("Sınıfta", "In class"))
                } else if self.hosting && self.connection.is_some() {
                    (true, self.tr("Paylaşım açık", "Sharing on"))
                } else {
                    (false, self.tr("Paylaşım kapalı", "Sharing off"))
                };
                let theme = if self.settings.dark { "☀" } else { "🌙" };
                let language = if self.settings.english { "TR" } else { "EN" };
                let mut toggle_theme = false;
                let mut toggle_language = false;
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if compact {
                        let small = |text: &str| {
                            egui::Button::new(RichText::new(text).size(12.))
                                .wrap_mode(egui::TextWrapMode::Extend)
                        };
                        toggle_language = ui.add_sized([40., 30.], small(language)).clicked();
                        toggle_theme = ui.add_sized([40., 30.], small(theme)).clicked();
                        ui.label(RichText::new("●").size(14.).color(if online {
                            p.green
                        } else {
                            p.line
                        }))
                        .on_hover_text(format!("{state} · {}", self.computer_name));
                        return;
                    }
                    ui.horizontal(|ui| {
                        let text = format!(
                            "{theme} {}",
                            if self.settings.dark {
                                self.tr("Açık", "Light")
                            } else {
                                self.tr("Koyu", "Dark")
                            }
                        );
                        toggle_theme = ui.add_sized([125., 32.], egui::Button::new(text)).clicked();
                        toggle_language = ui
                            .add_sized([50., 32.], egui::Button::new(language))
                            .clicked();
                    });
                    card(p).inner_margin(10).corner_radius(9).show(ui, |ui| {
                        ui.set_min_width(170.);
                        ui.horizontal(|ui| {
                            label(ui, "●", 12., if online { p.green } else { p.line });
                            ui.vertical(|ui| {
                                label(ui, state, 12., p.ink);
                                label(ui, &self.computer_name, 11., p.muted);
                            });
                        });
                    });
                });
                if toggle_theme {
                    self.settings.dark = !self.settings.dark;
                    self.persist_design();
                }
                if toggle_language {
                    self.settings.english = !self.settings.english;
                    self.persist_design();
                }
            });
    }
    fn home_design(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        ui.heading(self.tr("Bağlan veya paylaş", "Connect or share"));
        label(
            ui,
            self.tr(
                "Uçtan uca şifreli. Her bağlantıyı sen onaylarsın.",
                "End-to-end encrypted. You approve every connection.",
            ),
            14.,
            p.muted,
        );
        ui.add_space(18.);
        if ui.available_width() >= 650. {
            ui.columns(2, |columns| {
                self.share_card(&mut columns[0], ctx, p);
                self.connect_card(&mut columns[1], p);
            });
        } else {
            self.share_card(ui, ctx, p);
            ui.add_space(18.);
            self.connect_card(ui, p);
        }
        ui.add_space(20.);
        label(ui, self.status_text(), 12., p.muted);
    }
    fn share_card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        let sharing = self.hosting && self.connection.is_some();
        card(p).show(ui, |ui| {
            ui.horizontal(|ui| {
                monitor(ui, 30., p.accent);
                ui.vertical(|ui| {
                    label(
                        ui,
                        self.tr("Bu bilgisayarı paylaş", "Share this computer"),
                        16.,
                        p.ink,
                    );
                    label(
                        ui,
                        if sharing {
                            self.tr("Bağlantı bekleniyor…", "Waiting for a connection…")
                        } else {
                            self.tr("Paylaşım kapalı", "Sharing is off")
                        },
                        12.,
                        p.muted,
                    );
                });
            });
            ui.add_space(12.);
            if sharing {
                label(ui, self.tr("Bağlantı kodu", "Connection code"), 12., p.muted);
                egui::Frame::new()
                    .fill(p.field)
                    .stroke(Stroke::new(1.0_f32, p.line))
                    .corner_radius(10)
                    .inner_margin(14)
                    .show(ui, |ui| {
                        ui.add(
                            egui::Label::new(RichText::new(&self.secret).monospace().size(15.))
                                .wrap(),
                        );
                    });
                if !self.internet {
                    // Viewers on the same network need this address together with the code.
                    let address = self.shared_address();
                    ui.horizontal_wrapped(|ui| {
                        label(ui, self.tr("Bu bilgisayarın adresi:", "This computer's address:"), 12., p.muted);
                        ui.label(RichText::new(&address).monospace().strong());
                    });
                }
                let w = ((ui.available_width() - 8.) / 2.).max(80.);
                ui.horizontal_wrapped(|ui| {
                    if button(ui, self.tr("Kodu kopyala", "Copy code"), p, true, w).clicked() {
                        ctx.copy_text(self.secret.clone());
                    }
                    if !self.internet
                        && button(ui, self.tr("Davet bağlantısı", "Copy invite"), p, false, w)
                            .clicked()
                    {
                        match opendesk::invite::encode(
                            &self.shared_address(),
                            self.relay,
                            &self.secret,
                        ) {
                            Ok(code) => ctx.copy_text(code),
                            Err(e) => self.status = e,
                        }
                    }
                });
            } else {
                label(
                    ui,
                    self.tr(
                        "Paylaşımı başlat, kodu karşı tarafa ilet ve bağlanma isteğini onayla. Erişimi istediğin an durdurabilirsin.",
                        "Start sharing, send your code, then approve their request. You can stop access at any time.",
                    ),
                    13.,
                    p.muted,
                );
            }
            ui.add_space(8.);
            ui.separator();
            let control = self
                .tr(
                    "Fare ve klavye kontrolüne izin ver",
                    "Allow mouse and keyboard control",
                )
                .to_owned();
            ui.checkbox(&mut self.permissions.input, control);
            ui.separator();
            ui.horizontal(|ui| {
                label(ui, self.tr("Paylaşılan ekran", "Shared display"), 13., p.muted);
                let display = self.tr("Ekran", "Display").to_owned();
                egui::ComboBox::from_id_salt("home-monitor")
                    .selected_text(format!("{display} {}", self.monitor + 1))
                    .show_ui(ui, |ui| {
                        for i in 0..platform_count() {
                            ui.selectable_value(&mut self.monitor, i, format!("{display} {}", i + 1));
                        }
                    });
            });
            ui.add_space(10.);
            let hosting = self.hosting && (self.connection.is_some() || self.connecting.is_some());
            if hosting {
                if button(
                    ui,
                    self.tr("Paylaşımı durdur", "Stop sharing"),
                    p,
                    false,
                    ui.available_width(),
                )
                .clicked()
                {
                    self.stop();
                }
            } else if ui
                .add_enabled(
                    self.connection.is_none() && self.connecting.is_none(),
                    egui::Button::new(
                        RichText::new(self.tr("Paylaşımı başlat", "Start sharing"))
                            .color(Color32::WHITE),
                    )
                    .fill(p.accent)
                    .min_size(Vec2::new(ui.available_width(), 40.)),
                )
                .clicked()
            {
                self.retries = 0;
                self.connect_host();
            }
        });
    }
    fn connect_card(&mut self, ui: &mut egui::Ui, p: Palette) {
        card(p).show(ui, |ui| {
            ui.horizontal(|ui| {
                monitor(ui, 30., p.accent);
                ui.vertical(|ui| {
                    label(
                        ui,
                        self.tr("Uzak bilgisayara bağlan", "Connect to a computer"),
                        16.,
                        p.ink,
                    );
                    label(
                        ui,
                        self.tr(
                            "Davet kodu veya od2: daveti",
                            "Pairing code or od2: invitation",
                        ),
                        12.,
                        p.muted,
                    );
                });
            });
            ui.add_space(12.);
            ui.add(
                egui::TextEdit::singleline(&mut self.connection_input)
                    .font(egui::TextStyle::Monospace)
                    .desired_width(f32::INFINITY)
                    .min_size(Vec2::new(0., 40.))
                    .hint_text(if self.settings.english {
                        "Connection code or od2: invite"
                    } else {
                        "Bağlantı kodu veya od2: daveti"
                    }),
            );
            if !self.internet {
                ui.label(self.tr("Hedef adresi (IP:port)", "Target address (IP:port)"));
                ui.add(
                    egui::TextEdit::singleline(if self.relay {
                        &mut self.relay_address
                    } else {
                        &mut self.address
                    })
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("192.168.1.20:7878"),
                );
            }
            let enabled = !self.connection_input.trim().is_empty()
                && self.connection.is_none()
                && self.connecting.is_none();
            if ui
                .add_enabled(
                    enabled,
                    egui::Button::new(
                        RichText::new(self.tr("Bağlan →", "Connect →")).color(Color32::WHITE),
                    )
                    .fill(p.accent)
                    .min_size(Vec2::new(ui.available_width(), 40.)),
                )
                .clicked()
            {
                let input = self.connection_input.trim();
                if input.starts_with("od2:") {
                    match opendesk::invite::decode(input) {
                        Ok(invite) => {
                            self.internet = false;
                            self.relay = invite.relay;
                            if invite.relay {
                                self.relay_address = invite.endpoint
                            } else {
                                self.address = invite.endpoint
                            }
                            self.secret = invite.secret;
                            self.retries = 0;
                            self.connect_viewer();
                        }
                        Err(e) => self.status = e,
                    }
                } else {
                    self.secret = input.into();
                    self.retries = 0;
                    self.connect_viewer();
                }
            }
            label(
                ui,
                if self.internet {
                    self.tr(
                        "Kod yeterli: karşı taraf nerede olursa olsun IP gerekmez.",
                        "The code is enough: no IP needed, wherever the other computer is.",
                    )
                } else {
                    self.tr(
                        "Aynı ağda: karşı bilgisayarın IP adresini ve kodunu yaz.",
                        "Same network: enter the other computer's IP address and code.",
                    )
                },
                11.,
                p.muted,
            );
            ui.add_space(8.);
            ui.separator();
            ui.horizontal(|ui| {
                label(
                    ui,
                    self.tr("Kayıtlı cihazlar", "Saved devices"),
                    12.,
                    p.muted,
                );
                if ui.link(self.tr("Tümünü gör", "See all")).clicked() {
                    self.ui_page = Page::Devices;
                }
            });
            if self.settings.devices.is_empty() {
                label(
                    ui,
                    self.tr("Henüz kayıtlı cihaz yok.", "No saved devices yet."),
                    13.,
                    p.muted,
                );
            }
            let devices: Vec<_> = self.settings.devices.iter().take(3).cloned().collect();
            for device in devices {
                if ui
                    .add_sized(
                        [ui.available_width(), 36.],
                        egui::Button::new(format!("{}  →", device.name)),
                    )
                    .clicked()
                {
                    self.select_device(&device);
                }
            }
        });
    }
    fn select_device(&mut self, device: &settings::Device) {
        self.relay = device.relay;
        if device.relay {
            self.relay_address = device.address.clone()
        } else {
            self.address = device.address.clone()
        }
        self.ui_page = Page::Home;
        self.status = self
            .tr(
                "Adres yüklendi. Cihazın eşleştirme kodunu gir.",
                "Address loaded. Enter the device's pairing code.",
            )
            .into();
    }
    fn devices_design(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.horizontal(|ui| {
            ui.heading(self.tr("Cihazlar", "Devices"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if button(ui, self.tr("+ Cihaz ekle", "+ Add device"), p, true, 135.).clicked() {
                    self.add_device = !self.add_device;
                }
            });
        });
        label(
            ui,
            self.tr(
                "Adresler kaydedilir, eşleştirme anahtarları kaydedilmez.",
                "Addresses are saved; pairing secrets never are.",
            ),
            14.,
            p.muted,
        );
        ui.add_space(14.);
        ui.add(
            egui::TextEdit::singleline(&mut self.device_search)
                .desired_width(f32::INFINITY)
                .hint_text(if self.settings.english {
                    "Search devices"
                } else {
                    "Cihaz ara"
                }),
        );
        if self.add_device {
            card(p).show(ui, |ui| {
                ui.label(self.tr("Cihaz adı", "Device name"));
                ui.text_edit_singleline(&mut self.device_name);
                ui.label(self.tr("Adres", "Address"));
                ui.text_edit_singleline(if self.relay {
                    &mut self.relay_address
                } else {
                    &mut self.address
                });
                ui.checkbox(&mut self.relay, "Relay");
                if button(ui, self.tr("Adresi kaydet", "Save address"), p, true, 150.).clicked()
                    && !self.device_name.trim().is_empty()
                {
                    self.settings.devices.push(settings::Device {
                        name: self.device_name.trim().into(),
                        address: if self.relay {
                            self.relay_address.clone()
                        } else {
                            self.address.clone()
                        },
                        relay: self.relay,
                        favorite: false,
                    });
                    self.persist_design();
                    self.add_device = false;
                }
            });
        }
        let query = self.device_search.to_lowercase();
        let mut devices: Vec<_> = self
            .settings
            .devices
            .iter()
            .enumerate()
            .filter(|(_, d)| {
                format!("{} {}", d.name, d.address)
                    .to_lowercase()
                    .contains(&query)
            })
            .map(|(i, d)| (i, d.clone()))
            .collect();
        devices.sort_by_key(|(_, d)| !d.favorite);
        let mut remove = None;
        card(p).inner_margin(16).show(ui, |ui| {
            if devices.is_empty() {
                ui.add_space(20.);
                label(
                    ui,
                    self.tr("Eşleşen cihaz yok.", "No matching devices."),
                    13.,
                    p.muted,
                );
                ui.add_space(20.);
            }
            for (i, device) in devices {
                ui.horizontal(|ui| {
                    if ui.button(if device.favorite { "★" } else { "☆" }).clicked() {
                        self.settings.devices[i].favorite = !device.favorite;
                        self.persist_design();
                    }
                    monitor(ui, 28., p.muted);
                    ui.vertical(|ui| {
                        label(ui, &device.name, 14., p.ink);
                        ui.label(
                            RichText::new(&device.address)
                                .monospace()
                                .size(11.)
                                .color(p.muted),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(self.tr("Sil", "Remove")).clicked() {
                            remove = Some(i);
                        }
                        if ui.button(self.tr("Bağlan", "Connect")).clicked() {
                            self.select_device(&device);
                        }
                        label(
                            ui,
                            if device.relay {
                                "Relay"
                            } else {
                                self.tr("Doğrudan", "Direct")
                            },
                            11.,
                            p.muted,
                        );
                    });
                });
                ui.separator();
            }
        });
        if let Some(i) = remove {
            self.settings.devices.remove(i);
            self.persist_design();
        }
    }
    fn connecting_design(&mut self, ui: &mut egui::Ui, p: Palette) {
        ui.add_space(90.);
        ui.vertical_centered(|ui| {
            ui.spinner();
            ui.add_space(14.);
            ui.heading(self.tr("Bağlanılıyor", "Connecting"));
            label(
                ui,
                if self.internet {
                    self.tr("İnternet üzerinden, kod ile", "Over the Internet, by code")
                } else if self.relay {
                    &self.relay_address
                } else {
                    &self.address
                },
                14.,
                p.muted,
            );
            ui.add_space(20.);
            card(p).show(ui, |ui| {
                let transport = self.transport_ready;
                for (i, (tr, en)) in [
                    ("Şifreli kanal kuruluyor", "Setting up encrypted channel"),
                    ("Karşı tarafın onayı bekleniyor", "Waiting for approval"),
                    ("Görüntü alınıyor", "Receiving video"),
                ]
                .iter()
                .enumerate()
                {
                    ui.horizontal(|ui| {
                        let done = i == 0 && transport;
                        let active = if transport { i == 1 } else { i == 0 };
                        if done {
                            label(ui, "✓", 16., p.green);
                        } else if active {
                            ui.spinner();
                        } else {
                            label(ui, "○", 16., p.line);
                        }
                        label(
                            ui,
                            self.tr(tr, en),
                            13.,
                            if active || done { p.ink } else { p.muted },
                        );
                    });
                }
            });
            ui.add_space(12.);
            if button(ui, self.tr("Vazgeç", "Cancel"), p, false, 130.).clicked() {
                self.stop();
            }
            label(ui, self.status_text(), 12., p.muted);
        });
    }
    fn settings_design(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, p: Palette) {
        ui.set_max_width(660.);
        ui.heading(self.tr("Ayarlar", "Settings"));
        ui.add_space(10.);
        label(ui, self.tr("BAĞLANTI", "CONNECTION"), 12., p.muted);
        card(p).inner_margin(16).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(self.tr("Bağlantı yolu", "Route"));
            let before = (self.internet, self.relay);
            ui.horizontal_wrapped(|ui| {
                for (internet, relay, tr, en) in [
                    (true, false, "İnternet (yalnızca kod)", "Internet (code only)"),
                    (false, false, "Doğrudan (aynı ağ)", "Direct (same network)"),
                    (false, true, "Kendi relay sunucum", "My own relay server"),
                ] {
                    let selected = self.internet == internet && (internet || self.relay == relay);
                    if ui.selectable_label(selected, self.tr(tr, en)).clicked() {
                        self.internet = internet;
                        self.relay = relay;
                    }
                }
            });
            if before != (self.internet, self.relay) {
                self.settings.manual_route = !self.internet;
                self.settings.relay = self.relay;
                if let Err(e) = self.settings.save() {
                    self.status = e;
                }
            }
            ui.separator();
            if self.internet {
                ui.label(self.tr(
                    "Bilgisayarlar iroh ağı üzerinden yalnızca kodla bulunur. Mümkünse doğrudan bağlanır, değilse n0'ın ücretsiz relay sunucuları kullanılır. Oturum uçtan uca şifrelidir.",
                    "Computers find each other by code over the iroh network. Direct when possible, otherwise via n0's free public relays. Sessions stay end-to-end encrypted.",
                ));
            } else if self.relay {
                ui.label(self.tr("Relay adresi", "Relay address"));
                ui.text_edit_singleline(&mut self.relay_address);
            } else {
                ui.label(self.tr("Dinleme adresi / portu", "Listening address / port"));
                ui.text_edit_singleline(&mut self.bind);
                ui.label(self.tr(
                    "Davet için erişilebilir bilgisayar adresi",
                    "Reachable computer address for invitations",
                ));
                ui.text_edit_singleline(&mut self.address);
            }
            ui.checkbox(
                &mut self.auto_reconnect,
                if self.settings.english {
                    "Retry after disconnection (5 attempts)"
                } else {
                    "Bağlantı koparsa yeniden dene (5 deneme)"
                },
            );
        });
        ui.add_space(12.);
        label(ui, self.tr("GÖRÜNTÜ", "VIDEO"), 12., p.muted);
        card(p).inner_margin(16).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().interact_size.y = 20.;
            ui.spacing_mut().slider_width = 220.;
            ui.visuals_mut().widgets.inactive.bg_fill = p.line;
            ui.visuals_mut().widgets.inactive.fg_stroke = Stroke::new(2.0_f32, p.accent);
            ui.add(
                egui::Slider::new(&mut self.quality, 30..=90).text(if self.settings.english {
                    "Image quality"
                } else {
                    "Görüntü kalitesi"
                }),
            );
            ui.add(
                egui::Slider::new(&mut self.fps, 1..=30).text(if self.settings.english {
                    "Frames / second"
                } else {
                    "Kare / saniye"
                }),
            );
            ui.checkbox(
                &mut self.adaptive,
                if self.settings.english {
                    "Adapt quality to connection speed"
                } else {
                    "Kaliteyi bağlantı hızına göre ayarla"
                },
            );
            ui.add(egui::Slider::new(&mut self.max_width, 640..=3840).text(
                if self.settings.english {
                    "Maximum width"
                } else {
                    "En fazla genişlik"
                },
            ));
        });
        ui.add_space(12.);
        label(
            ui,
            self.tr("DOSYALAR VE KAYIT", "FILES AND RECORDING"),
            12.,
            p.muted,
        );
        card(p).inner_margin(16).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.label(self.tr("Gelen dosyalar", "Incoming files"));
            ui.text_edit_singleline(&mut self.inbox);
            ui.label(self.tr("Video kayıt dosyası (.avi)", "Video recording file (.avi)"));
            ui.text_edit_singleline(&mut self.record_path);
        });
        ui.add_space(12.);
        label(ui, self.tr("KULLANIM", "USAGE"), 12., p.muted);
        card(p).inner_margin(16).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let before = self.settings.role;
            let mut role = before.unwrap_or(settings::Role::Personal);
            ui.horizontal_wrapped(|ui| {
                ui.label(self.tr("Bu bilgisayar", "This computer is"));
                for (value, tr, en) in [
                    (settings::Role::Personal, "Bireysel", "Personal"),
                    (settings::Role::Teacher, "Öğretmen", "Teacher"),
                    (settings::Role::Student, "Öğrenci", "Student"),
                ] {
                    let text = self.tr(tr, en).to_owned();
                    ui.selectable_value(&mut role, value, text);
                }
            });
            label(
                ui,
                self.tr(
                    "Uygulamanın hangi ekranla açılacağını belirler; tüm özellikler her rolde kullanılabilir.",
                    "Sets the start screen; every feature stays available in every role.",
                ),
                11.,
                p.muted,
            );
            if Some(role) != before {
                self.settings.role = Some(role);
                self.persist_design();
            }
        });
        ui.add_space(12.);
        label(ui, self.tr("GÖRÜNÜM", "APPEARANCE"), 12., p.muted);
        card(p).inner_margin(16).show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let old = (
                self.settings.dark,
                self.settings.english,
                self.settings.logo,
            );
            ui.horizontal(|ui| {
                ui.label(self.tr("Tema", "Theme"));
                ui.selectable_value(
                    &mut self.settings.dark,
                    false,
                    if self.settings.english {
                        "Light"
                    } else {
                        "Açık"
                    },
                );
                ui.selectable_value(
                    &mut self.settings.dark,
                    true,
                    if self.settings.english {
                        "Dark"
                    } else {
                        "Koyu"
                    },
                );
            });
            ui.horizontal(|ui| {
                ui.label(self.tr("Dil", "Language"));
                ui.selectable_value(&mut self.settings.english, false, "Türkçe");
                ui.selectable_value(&mut self.settings.english, true, "English");
            });
            ui.horizontal(|ui| {
                ui.label("Logo");
                for i in 0..3 {
                    if ui
                        .selectable_label(
                            self.settings.logo == i,
                            format!("{}", char::from(b'A' + i)),
                        )
                        .clicked()
                    {
                        self.settings.logo = i;
                    }
                    logo(ui, 24., i, p);
                }
            });
            if old
                != (
                    self.settings.dark,
                    self.settings.english,
                    self.settings.logo,
                )
            {
                self.persist_design();
            }
        });
        ui.add_space(12.);
        ui.collapsing(
            self.tr(
                "Gelişmiş ayarlar ve güvenlik",
                "Advanced settings and security",
            ),
            |ui| self.extra_controls(ui, ctx),
        );
        ui.add_space(16.);
        label(
            ui,
            &format!(
                "Opendesk {} · MIT · {}",
                env!("CARGO_PKG_VERSION"),
                self.tr("Açık kaynak", "Open source")
            ),
            12.,
            p.muted,
        );
        label(ui, self.status_text(), 12., p.muted);
    }
}
impl Desk {
    fn request_design(&mut self, ctx: &egui::Context, p: Palette) {
        if let Some(peer) = self.incoming.clone() {
            egui::Modal::new(egui::Id::new("real-incoming-request")).show(ctx, |ui| {
                ui.set_width(380.);
                ui.heading(self.tr("Bağlantı isteği", "Connection request"));
                ui.add_space(8.);
                label(ui, &peer, 16., p.ink);
                label(
                    ui,
                    self.tr(
                        "bu bilgisayara bağlanmak istiyor.",
                        "wants to connect to this computer.",
                    ),
                    13.,
                    p.muted,
                );
                ui.separator();
                ui.checkbox(
                    &mut self.permissions.screen,
                    if self.settings.english {
                        "View screen"
                    } else {
                        "Ekranı görme"
                    },
                );
                ui.checkbox(
                    &mut self.permissions.input,
                    if self.settings.english {
                        "Mouse and keyboard"
                    } else {
                        "Fare ve klavye"
                    },
                );
                ui.checkbox(
                    &mut self.permissions.clipboard,
                    if self.settings.english {
                        "Clipboard"
                    } else {
                        "Pano"
                    },
                );
                ui.checkbox(
                    &mut self.permissions.files,
                    if self.settings.english {
                        "File transfer"
                    } else {
                        "Dosya aktarımı"
                    },
                );
                ui.checkbox(
                    &mut self.permissions.audio,
                    if self.settings.english {
                        "System audio"
                    } else {
                        "Sistem sesi"
                    },
                );
                ui.add_space(8.);
                label(
                    ui,
                    self.tr(
                        "Kabul edersen seçili ekranın paylaşılır. İstediğin an durdurabilirsin.",
                        "Accepting shares your selected display. You can stop at any time.",
                    ),
                    12.,
                    p.muted,
                );
                ui.horizontal(|ui| {
                    if button(ui, self.tr("Reddet", "Decline"), p, false, 175.).clicked()
                        && self.send(Command::Reject)
                    {
                        self.incoming = None;
                    }
                    if button(ui, self.tr("Kabul et", "Accept"), p, true, 175.).clicked()
                        && self.send(Command::AcceptPermissions(self.permissions))
                    {
                        self.incoming = None;
                    }
                });
            });
        }
    }
    fn session_design(&mut self, ctx: &egui::Context, p: Palette) {
        egui::TopBottomPanel::top("session-toolbar")
            .frame(
                egui::Frame::new()
                    .fill(p.card)
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    label(ui, "●", 12., p.green);
                    label(ui, &self.peer_label, 14., p.ink);
                    label(ui, self.tr("Şifreli", "Encrypted"), 12., p.green);
                    ui.separator();
                    if self.hosting {
                        egui::ComboBox::from_id_salt("session-monitor")
                            .selected_text(format!(
                                "{} {}",
                                self.tr("Ekran", "Display"),
                                self.monitor + 1
                            ))
                            .show_ui(ui, |ui| {
                                for i in 0..platform_count() {
                                    if ui
                                        .selectable_label(
                                            self.monitor == i,
                                            format!(
                                                "{} {}",
                                                if self.settings.english {
                                                    "Display"
                                                } else {
                                                    "Ekran"
                                                },
                                                i + 1
                                            ),
                                        )
                                        .clicked()
                                    {
                                        self.finish_recording();
                                        self.monitor = i;
                                        self.set_input(self.control);
                                    }
                                }
                            });
                    } else {
                        let mut selection = None;
                        egui::ComboBox::from_id_salt("session-monitor")
                            .selected_text(format!(
                                "{} {}",
                                self.tr("Ekran", "Display"),
                                self.monitor + 1
                            ))
                            .show_ui(ui, |ui| {
                                for monitor in &self.monitors {
                                    if ui
                                        .selectable_label(self.monitor == monitor.id, &monitor.name)
                                        .clicked()
                                    {
                                        selection = Some(monitor.id);
                                    }
                                }
                            });
                        if let Some(id) = selection
                            && self.send(Command::SelectMonitor(id))
                        {
                            self.finish_recording();
                            self.monitor = id;
                        }
                    }
                    let before = (self.quality, self.fps);
                    ui.menu_button(self.tr("Kalite", "Quality"), |ui| {
                        ui.checkbox(
                            &mut self.adaptive,
                            if self.settings.english {
                                "Automatic"
                            } else {
                                "Otomatik"
                            },
                        );
                        ui.add(egui::Slider::new(&mut self.quality, 30..=90));
                        ui.add(egui::Slider::new(&mut self.fps, 1..=30).text("FPS"));
                        ui.add(egui::Slider::new(&mut self.max_width, 640..=3840).text("px"));
                    });
                    // The host encodes the video, so a viewer's manual choice is sent there.
                    if !self.hosting && before != (self.quality, self.fps) {
                        self.pending_feedback = true;
                    }
                    if self.pending_feedback && !ctx.is_using_pointer() {
                        self.pending_feedback = !self.send(Command::VideoFeedback {
                            max_fps: self.fps.clamp(1, 30) as u8,
                            quality: self.quality,
                        });
                    }
                    if !self.hosting
                        && ui
                            .selectable_label(
                                self.fit_to_window,
                                self.tr("Pencereye sığdır", "Fit to window"),
                            )
                            .clicked()
                    {
                        self.fit_to_window = !self.fit_to_window;
                    }
                    for (tab, tr, en) in [
                        (Tab::Chat, "Sohbet", "Chat"),
                        (Tab::Files, "Dosyalar", "Files"),
                        (Tab::Clipboard, "Pano", "Clipboard"),
                    ] {
                        if ui
                            .selectable_label(self.ui_tab == Some(tab), self.tr(tr, en))
                            .clicked()
                        {
                            self.ui_tab = if self.ui_tab == Some(tab) {
                                None
                            } else {
                                Some(tab)
                            };
                        }
                    }
                    if !self.hosting
                        && ui
                            .button(if self.recorder.is_some() {
                                self.tr("● Kayıtta", "● Recording")
                            } else {
                                self.tr("○ Kaydet", "○ Record")
                            })
                            .clicked()
                    {
                        if self.recorder.is_some() {
                            self.finish_recording();
                        } else if let Some((w, h)) = self.frame_size {
                            match crate::recording::Recorder::start(
                                std::path::Path::new(&self.record_path),
                                w,
                                h,
                                self.fps as u32,
                            ) {
                                Ok(r) => self.recorder = Some(r),
                                Err(e) => self.status = e,
                            }
                        }
                    }
                    if ui
                        .button(if self.fullscreen {
                            self.tr("Tam ekrandan çık (Esc)", "Exit fullscreen (Esc)")
                        } else {
                            self.tr("Tam ekran", "Fullscreen")
                        })
                        .clicked()
                    {
                        self.fullscreen = !self.fullscreen;
                        ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(self.fullscreen));
                    }
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(self.tr("Bitir", "End")).color(Color32::WHITE),
                            )
                            .fill(p.red),
                        )
                        .clicked()
                    {
                        self.stop();
                    }
                    ui.menu_button(self.tr("İzinler", "Permissions"), |ui| {
                        let old = self.permissions;
                        ui.add_enabled_ui(self.hosting, |ui| {
                            ui.checkbox(
                                &mut self.permissions.screen,
                                if self.settings.english {
                                    "Screen"
                                } else {
                                    "Ekran"
                                },
                            );
                            ui.checkbox(
                                &mut self.permissions.input,
                                if self.settings.english {
                                    "Input"
                                } else {
                                    "Kontrol"
                                },
                            );
                            ui.checkbox(
                                &mut self.permissions.clipboard,
                                if self.settings.english {
                                    "Clipboard"
                                } else {
                                    "Pano"
                                },
                            );
                            ui.checkbox(
                                &mut self.permissions.files,
                                if self.settings.english {
                                    "Files"
                                } else {
                                    "Dosyalar"
                                },
                            );
                            ui.checkbox(
                                &mut self.permissions.audio,
                                if self.settings.english {
                                    "Audio"
                                } else {
                                    "Ses"
                                },
                            );
                        });
                        if old != self.permissions {
                            if !self.send(Command::SetPermissions(self.permissions)) {
                                self.permissions = old;
                            } else {
                                self.control = self.permissions.input;
                                self.set_input(self.control);
                            }
                        }
                    });
                });
            });
        if !self.fullscreen {
            egui::TopBottomPanel::bottom("session-metrics")
                .frame(egui::Frame::new().fill(p.side).inner_margin(8))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "{:.1} fps  ·  {}",
                                self.actual_fps,
                                if self.internet {
                                    "internet"
                                } else if self.relay {
                                    "relay"
                                } else {
                                    self.tr("doğrudan", "direct")
                                }
                            ))
                            .monospace()
                            .size(11.)
                            .color(p.muted),
                        );
                        label(ui, &self.video_stats, 11., p.muted);
                        if self.recorder.is_some() {
                            label(ui, self.tr("● KAYITTA", "● RECORDING"), 11., p.red);
                        }
                    });
                });
        }
        if let Some(tab) = self.ui_tab.filter(|_| !self.fullscreen) {
            let width = ctx.screen_rect().width();
            egui::SidePanel::right("session-tools")
                .default_width(320_f32.min(width * 0.35))
                .width_range(240.0..=(width * 0.5).max(260.))
                .resizable(true)
                .frame(egui::Frame::new().fill(p.card).inner_margin(14))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        for (t, tr, en) in [
                            (Tab::Chat, "Sohbet", "Chat"),
                            (Tab::Files, "Dosyalar", "Files"),
                            (Tab::Clipboard, "Pano", "Clipboard"),
                        ] {
                            if ui.selectable_label(tab == t, self.tr(tr, en)).clicked() {
                                self.ui_tab = Some(t);
                            }
                        }
                        if ui.small_button("×").clicked() {
                            self.ui_tab = None;
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .stick_to_bottom(tab == Tab::Chat)
                        .show(ui, |ui| match tab {
                            Tab::Chat => self.chat_design(ui, p),
                            Tab::Files => self.files_design(ui, p),
                            Tab::Clipboard => self.clipboard_design(ui, p),
                        });
                });
        }
        let dropped: Vec<_> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for path in dropped {
            if !self.permissions.files {
                self.status = self
                    .tr(
                        "Dosya aktarım izni kapalı.",
                        "File transfer is not permitted.",
                    )
                    .into();
            } else if self.send(Command::SendPath(path.clone())) {
                self.outgoing = path.display().to_string();
                self.ui_tab = Some(Tab::Files);
            } else {
                self.status = self
                    .tr("Aktarım kuyruğu meşgul.", "Transfer queue busy.")
                    .into();
            }
        }
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(40, 47, 61))
                    .inner_margin(16),
            )
            .show(ctx, |ui| {
                ui.visuals_mut().override_text_color = Some(Color32::from_rgb(228, 234, 244));
                self.remote_canvas(ui, ctx);
            });
    }
    fn chat_design(&mut self, ui: &mut egui::Ui, p: Palette) {
        if self.messages.is_empty() {
            label(
                ui,
                self.tr("Henüz mesaj yok.", "No messages yet."),
                13.,
                p.muted,
            );
        }
        for message in &self.messages {
            let mine = message.starts_with("You: ");
            let text = message
                .strip_prefix("You: ")
                .or_else(|| message.strip_prefix("Peer: "))
                .unwrap_or(message);
            ui.with_layout(
                if mine {
                    egui::Layout::right_to_left(egui::Align::Min)
                } else {
                    egui::Layout::left_to_right(egui::Align::Min)
                },
                |ui| {
                    egui::Frame::new()
                        .fill(if mine { p.accent } else { p.field })
                        .corner_radius(12)
                        .inner_margin(10)
                        .show(ui, |ui| {
                            ui.set_max_width(220.);
                            label(ui, text, 13., if mine { Color32::WHITE } else { p.ink });
                        });
                },
            );
        }
        ui.add_space(16.);
        let field = ui.add(
            egui::TextEdit::multiline(&mut self.chat)
                .desired_width(f32::INFINITY)
                .desired_rows(2)
                .hint_text(if self.settings.english {
                    "Type a message… (Enter sends, Shift+Enter new line)"
                } else {
                    "Mesaj yaz… (Enter gönderir, Shift+Enter yeni satır)"
                }),
        );
        // Enter sends; Shift+Enter keeps the newline the text field inserted.
        let enter = field.has_focus()
            && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if enter {
            while self.chat.ends_with('\n') || self.chat.ends_with('\r') {
                self.chat.pop();
            }
        }
        if (button(ui, self.tr("Gönder", "Send"), p, true, ui.available_width()).clicked() || enter)
            && !self.chat.trim().is_empty()
            && self.chat.len() <= 4096
            && self.send(Command::Chat(self.chat.clone()))
        {
            self.messages.push(format!("You: {}", self.chat));
            self.chat.clear();
            if self.messages.len() > 100 {
                self.messages.remove(0);
            }
        }
    }
    fn files_design(&mut self, ui: &mut egui::Ui, p: Palette) {
        label(
            ui,
            self.tr("Dosya veya klasör yolu", "File or folder path"),
            13.,
            p.ink,
        );
        ui.add(egui::TextEdit::singleline(&mut self.outgoing).desired_width(f32::INFINITY));
        if ui
            .add_enabled(
                self.permissions.files,
                egui::Button::new(self.tr("Dosya / klasör gönder", "Send file / folder"))
                    .min_size(Vec2::new(ui.available_width(), 40.)),
            )
            .clicked()
            && !self.send(Command::SendPath(std::path::PathBuf::from(&self.outgoing)))
        {
            self.status = self
                .tr("Aktarım kuyruğu meşgul.", "Transfer queue busy.")
                .into();
        }
        label(ui, &self.transfer_status, 12., p.muted);
        label(
            ui,
            self.tr(
                "Kesilen aktarımı sürdürmek için aynı yolu yeniden gönder ve tekrar kabul et.",
                "To resume an interrupted transfer, resend the same path and accept again.",
            ),
            12.,
            p.muted,
        );
        if let Some(manifest) = &self.transfer_offer {
            ui.separator();
            label(
                ui,
                self.tr("Gelen aktarım", "Incoming transfer"),
                14.,
                p.ink,
            );
            label(
                ui,
                &format!(
                    "{} {} · {} bytes",
                    manifest.entries.len(),
                    self.tr("dosya", "files"),
                    manifest.total
                ),
                12.,
                p.muted,
            );
        }
        if self.transfer_offer.is_some() {
            ui.text_edit_singleline(&mut self.inbox);
            if button(
                ui,
                self.tr("Kabul et / sürdür", "Accept / resume"),
                p,
                true,
                ui.available_width(),
            )
            .clicked()
                && self.send(Command::ReceiveTransfer {
                    directory: std::path::PathBuf::from(&self.inbox),
                })
            {
                self.transfer_offer = None;
            }
            if ui.button(self.tr("Reddet", "Decline")).clicked()
                && self.send(Command::RejectTransfer)
            {
                self.transfer_offer = None;
            }
        }
        if let Some((name, bytes)) = &self.incoming_file {
            label(ui, &format!("{name} · {} bytes", bytes.len()), 13., p.ink);
        }
        if self.incoming_file.is_some() {
            ui.text_edit_singleline(&mut self.inbox);
            if ui.button(self.tr("Kaydet", "Save")).clicked()
                && let Some((name, bytes)) = self.incoming_file.take()
            {
                match opendesk::transfer::save_received(
                    std::path::Path::new(&self.inbox),
                    &name,
                    &bytes,
                ) {
                    Ok(path) => self.status = format!("{}", path.display()),
                    Err(e) => self.status = e,
                }
            }
            if ui.button(self.tr("Reddet", "Decline")).clicked() {
                self.incoming_file = None;
            }
        }
    }
    fn clipboard_design(&mut self, ui: &mut egui::Ui, p: Palette) {
        label(
            ui,
            self.tr(
                "Pano otomatik eşitlenmez: sen gönderirsin, karşı taraf onaylar. En fazla 8 KiB metin.",
                "Clipboard is never synced automatically: you send it, they approve. Text up to 8 KiB.",
            ),
            13.,
            p.muted,
        );
        if ui
            .add_enabled(
                self.permissions.clipboard,
                egui::Button::new(self.tr("Panomu gönder", "Send my clipboard"))
                    .min_size(Vec2::new(ui.available_width(), 40.)),
            )
            .clicked()
        {
            match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                Ok(text) if text.len() <= 8192 => {
                    if !self.send(Command::Clipboard(text)) {
                        self.status = self
                            .tr("Pano kuyruğu meşgul.", "Clipboard queue busy.")
                            .into();
                    }
                }
                Ok(_) => {
                    self.status = self
                        .tr("Pano 8 KiB sınırını aşıyor.", "Clipboard exceeds 8 KiB.")
                        .into()
                }
                Err(e) => self.status = e.to_string(),
            }
        }
        if let Some(text) = &self.received_clipboard {
            ui.separator();
            label(ui, self.tr("Gelen pano", "Received clipboard"), 14., p.ink);
            egui::Frame::new()
                .fill(p.field)
                .corner_radius(8)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(text.chars().take(512).collect::<String>())
                            .monospace()
                            .size(12.),
                    );
                });
        }
        if self.received_clipboard.is_some() {
            if button(
                ui,
                self.tr("Uygula", "Apply"),
                p,
                true,
                ui.available_width(),
            )
            .clicked()
                && let Some(text) = self.received_clipboard.take()
            {
                match arboard::Clipboard::new().and_then(|mut c| c.set_text(text)) {
                    Ok(()) => self.status = self.tr("Pano uygulandı.", "Clipboard applied.").into(),
                    Err(e) => self.status = e.to_string(),
                }
            }
            if ui.button(self.tr("At", "Discard")).clicked() {
                self.received_clipboard = None;
            }
        }
    }
}
fn platform_count() -> usize {
    crate::platform::display_list()
        .map(|m| m.len())
        .unwrap_or(1)
}
