use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub relay: bool,
    pub favorite: bool,
}
/// Session preferences remembered between launches.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub auto_reconnect: bool,
    pub adaptive: bool,
    pub quality: u8,
    pub fps: u64,
    pub max_width: u32,
    pub bind: String,
    pub inbox: String,
    pub record_path: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            auto_reconnect: false,
            adaptive: true,
            quality: 65,
            fps: 6,
            max_width: 1920,
            bind: "0.0.0.0:7878".into(),
            inbox: "Opendesk-inbox".into(),
            record_path: "Opendesk-recording.avi".into(),
        }
    }
}
impl Preferences {
    /// Clamps values edited by hand in settings.json into the ranges the UI offers.
    pub fn sanitized(mut self) -> Self {
        self.quality = self.quality.clamp(30, 90);
        self.fps = self.fps.clamp(1, 30);
        self.max_width = self.max_width.clamp(640, 3840);
        let defaults = Self::default();
        for (value, default) in [
            (&mut self.bind, defaults.bind),
            (&mut self.inbox, defaults.inbox),
            (&mut self.record_path, defaults.record_path),
        ] {
            if value.trim().is_empty() {
                *value = default;
            }
        }
        self
    }
}
/// How this computer is mainly used; chosen at installation or first launch.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Personal,
    Teacher,
    Student,
}
impl Role {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "personal" | "bireysel" => Some(Self::Personal),
            "teacher" | "ogretmen" | "öğretmen" => Some(Self::Teacher),
            "student" | "ogrenci" | "öğrenci" => Some(Self::Student),
            _ => None,
        }
    }
}
/// Classroom details remembered for the next lesson.
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ClassPrefs {
    pub class_name: String,
    pub class_code: String,
    pub student_name: String,
    pub last_address: String,
    pub last_code: String,
    /// Students rejoin the last class automatically when Opendesk starts.
    pub autojoin: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub english: bool,
    #[serde(default)]
    pub logo: u8,
    #[serde(default)]
    pub devices: Vec<Device>,
    #[serde(default)]
    pub update_endpoint: String,
    #[serde(default)]
    pub relay: bool,
    #[serde(default)]
    pub relay_address: String,
    /// False (the default) connects by code over the Internet via iroh.
    #[serde(default)]
    pub manual_route: bool,
    #[serde(default)]
    pub preferences: Preferences,
    #[serde(default)]
    pub role: Option<Role>,
    #[serde(default)]
    pub class: ClassPrefs,
}
impl Settings {
    /// Role chosen in the Windows installer, stored beside the executable so it also
    /// applies when an administrator installs for every user of a lab computer.
    pub fn installed_role() -> Option<Role> {
        let exe = std::env::current_exe().ok()?;
        let text = std::fs::read_to_string(exe.parent()?.join("role.txt")).ok()?;
        Role::parse(&text)
    }
    fn path() -> PathBuf {
        std::env::var_os(if cfg!(windows) {
            "APPDATA"
        } else {
            "XDG_CONFIG_HOME"
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(".config")
        })
        .join("Opendesk")
        .join("settings.json")
    }
    pub fn load() -> Self {
        std::fs::read(Self::path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        std::fs::create_dir_all(path.parent().ok_or("Invalid settings path")?)
            .map_err(|e| e.to_string())?;
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        // Write then rename so a crash or power loss never leaves a truncated file.
        let temporary = path.with_extension("json.tmp");
        std::fs::write(&temporary, data).map_err(|e| e.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|e| e.to_string())
    }
}
