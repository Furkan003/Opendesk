use std::{path::PathBuf, time::Duration};
fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--apply")) {
        return Err("Usage: opendesk-updater --apply STAGING_DIRECTORY".into());
    }
    let staging = PathBuf::from(args.next().ok_or("Missing staging directory")?);
    if args.next().is_some() {
        return Err("Unexpected argument".into());
    }
    let payload = opendesk::update::verify_staged(&staging)?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let directory = exe.parent().ok_or("Missing application directory")?;
    let manifest = opendesk::update::verify(
        &std::fs::read(staging.join("manifest.json")).map_err(|e| e.to_string())?,
    )?;
    let installed = std::fs::read_to_string(directory.join("installed-version.txt"))
        .unwrap_or_else(|_| env!("CARGO_PKG_VERSION").into());
    if semver::Version::parse(&manifest.version).map_err(|e| e.to_string())?
        <= semver::Version::parse(installed.trim()).map_err(|e| e.to_string())?
    {
        return Err("Installed version is newer than or equal to staged update".into());
    }
    let target = directory.join(if cfg!(windows) {
        "opendesk.exe"
    } else {
        "opendesk"
    });
    if !target.is_file() {
        return Err("Updater must be installed beside the Opendesk application".into());
    }
    let backup = directory.join("opendesk.update-backup");
    if backup.exists() {
        return Err("Previous update backup exists; resolve it before another update".into());
    }
    std::thread::sleep(Duration::from_secs(2));
    let mut moved = false;
    for _ in 0..120 {
        match std::fs::rename(&target, &backup) {
            Ok(()) => {
                moved = true;
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
    if !moved {
        return Err("Application is still running; close it before applying update".into());
    }
    let result = std::fs::copy(&payload, &target)
        .map_err(|e| e.to_string())
        .and_then(|_| {
            opendesk::update::verify_asset(&target, &manifest)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
            }
            std::process::Command::new(&target)
                .spawn()
                .map_err(|e| e.to_string())?;
            Ok(())
        });
    if let Err(e) = result {
        let _ = std::fs::remove_file(&target);
        std::fs::rename(&backup, &target)
            .map_err(|rollback| format!("Update failed: {e}; rollback failed: {rollback}"))?;
        return Err(format!("Update failed; previous app restored: {e}"));
    }
    std::fs::remove_file(backup).map_err(|e| e.to_string())?;
    std::fs::write(directory.join("installed-version.txt"), manifest.version)
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
