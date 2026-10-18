#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
use eframe::egui;
struct Setup {
    directory: String,
    shortcut: bool,
    autostart: bool,
    status: String,
}
impl Default for Setup {
    fn default() -> Self {
        Self {
            directory: opendesk::installer::default_directory()
                .to_string_lossy()
                .into(),
            shortcut: false,
            autostart: false,
            status: "Extract the full bundle before installation.".into(),
        }
    }
}
impl eframe::App for Setup {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Install Opendesk");
            ui.label("Per-user installation. No administrator account required.");
            ui.label("Installation directory");
            ui.text_edit_singleline(&mut self.directory);
            ui.checkbox(&mut self.shortcut, "Create application shortcut");
            ui.checkbox(
                &mut self.autostart,
                "Start visible Opendesk window when I sign in",
            );
            ui.label("Autostart does not grant access or start listening.");
            if ui.button("Install").clicked() {
                self.status =
                    match std::env::current_exe()
                        .map_err(|e| e.to_string())
                        .and_then(|exe| {
                            opendesk::installer::install(
                                exe.parent().ok_or("Setup folder unavailable")?,
                                std::path::Path::new(&self.directory),
                                self.shortcut,
                                self.autostart,
                            )
                        }) {
                        Ok(()) => {
                            "Installed successfully. Launch Opendesk from the selected folder."
                                .into()
                        }
                        Err(e) => e,
                    };
            }
            ui.label(&self.status);
        });
    }
}
fn main() -> eframe::Result<()> {
    eframe::run_native(
        "Opendesk setup",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default().with_inner_size([580.0, 330.0]),
            ..Default::default()
        },
        Box::new(|_| Ok(Box::new(Setup::default()))),
    )
}
