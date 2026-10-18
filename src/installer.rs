use std::path::{Path, PathBuf};
pub fn default_directory() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join("Opendesk")
    } else {
        std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default()
            .join(".local/share/Opendesk")
    }
}
fn filename(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.into()
    }
}
pub fn install(
    source: &Path,
    destination: &Path,
    shortcut: bool,
    autostart: bool,
) -> Result<(), String> {
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    if !source.join(filename("opendesk")).is_file() {
        return Err(
            "Setup requires the complete extracted bundle beside it (opendesk executable missing)"
                .into(),
        );
    }
    std::fs::create_dir_all(destination).map_err(|e| e.to_string())?;
    let destination = destination.canonicalize().map_err(|e| e.to_string())?;
    if source == destination {
        return Err("Choose an installation directory different from the extracted bundle".into());
    }
    for name in ["opendesk", "opendesk-relay", "opendesk-updater"] {
        let file = filename(name);
        let origin = source.join(&file);
        if !origin.is_file() {
            return Err(format!("Bundle component missing: {file}"));
        }
        let target = destination.join(&file);
        if target.exists() {
            return Err(format!(
                "Installation file already exists: {}. Use signed updates for an existing installation.",
                target.display()
            ));
        }
    }
    for name in ["opendesk", "opendesk-relay", "opendesk-updater"] {
        let file = filename(name);
        std::fs::copy(source.join(&file), destination.join(&file)).map_err(|e| e.to_string())?;
    }
    for relative in [
        "run-opendesk.sh",
        "lib/libxdo.so.3",
        "lib/COPYRIGHT",
        "lib/COPYING",
        "lib/copyright",
    ] {
        let origin = source.join(relative);
        if origin.is_file() {
            let target = destination.join(relative);
            if target.exists() {
                return Err(format!(
                    "Installation asset already exists: {}",
                    target.display()
                ));
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::copy(origin, target).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(
        destination.join("installed-version.txt"),
        env!("CARGO_PKG_VERSION"),
    )
    .map_err(|e| e.to_string())?;
    shortcuts(&destination, shortcut, autostart)?;
    Ok(())
}
#[cfg(windows)]
fn shortcuts(destination: &Path, shortcut: bool, autostart: bool) -> Result<(), String> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .ok_or("APPDATA unavailable")?;
    let exe = destination.join("opendesk.exe");
    let mut folders = Vec::new();
    if shortcut {
        folders.push(base.join("Microsoft/Windows/Start Menu/Programs"));
    }
    if autostart {
        folders.push(base.join("Microsoft/Windows/Start Menu/Programs/Startup"));
    }
    for folder in folders {
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        let quote = |s: &Path| s.to_string_lossy().replace('\'', "''");
        let script = format!(
            "$w=New-Object -ComObject WScript.Shell; $s=$w.CreateShortcut('{}'); $s.TargetPath='{}'; $s.WorkingDirectory='{}'; $s.Save()",
            quote(&folder.join("Opendesk.lnk")),
            quote(&exe),
            quote(destination)
        );
        let status = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            return Err("Could not create Windows shortcut".into());
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn shortcuts(destination: &Path, shortcut: bool, autostart: bool) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME unavailable")?;
    let exe = destination
        .join(if destination.join("run-opendesk.sh").is_file() {
            "run-opendesk.sh"
        } else {
            "opendesk"
        })
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Opendesk\nExec=\"{exe}\"\nTerminal=false\nCategories=Network;RemoteAccess;\n"
    );
    let mut folders = Vec::new();
    if shortcut {
        folders.push(home.join(".local/share/applications"));
    }
    if autostart {
        folders.push(home.join(".config/autostart"));
    }
    for folder in folders {
        std::fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
        std::fs::write(folder.join("opendesk.desktop"), &entry).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installs_temporary_fake_bundle_and_preserves_existing_files() {
        let base = std::env::temp_dir().join(format!(
            "opendesk-installer-test-{:016x}",
            rand::random::<u64>()
        ));
        let source = base.join("bundle");
        let destination = base.join("installed");
        std::fs::create_dir_all(&source).unwrap();
        for name in ["opendesk", "opendesk-relay", "opendesk-updater"] {
            std::fs::write(source.join(filename(name)), b"FAKE TEST EXECUTABLE").unwrap();
        }
        install(&source, &destination, false, false).unwrap();
        assert_eq!(
            std::fs::read(destination.join(filename("opendesk"))).unwrap(),
            b"FAKE TEST EXECUTABLE"
        );
        assert!(install(&source, &destination, false, false).is_err());
        assert_eq!(
            std::fs::read(destination.join(filename("opendesk"))).unwrap(),
            b"FAKE TEST EXECUTABLE"
        );
        assert!(install(&source, &source, false, false).is_err());
        std::fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn rejects_missing_bundle_before_creating_destination() {
        let base = std::env::temp_dir().join(format!(
            "opendesk-installer-missing-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir(&base).unwrap();
        let destination = base.join("destination");
        assert!(install(&base, &destination, false, false).is_err());
        assert!(!destination.exists());
        std::fs::remove_dir(base).unwrap();
    }
}
