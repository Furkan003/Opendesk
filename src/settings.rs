use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Device {
    pub name: String,
    pub address: String,
    pub relay: bool,
    pub favorite: bool,
}
#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub dark: bool,
    #[serde(default)]
    pub english: bool,
    #[serde(default)]
    pub logo: u8,
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
}
impl Settings {
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
        std::fs::write(path, data).map_err(|e| e.to_string())
    }
}
