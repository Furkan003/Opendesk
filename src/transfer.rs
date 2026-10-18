//! Files are saved only after the receiver approves the destination locally.
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const MAX_FILE_SIZE: usize = 16 * 1024 * 1024;

fn valid_name(name: &str) -> Result<(), String> {
    if name.is_empty()
        || name.len() > 255
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:<>\"|?*".contains(c))
    {
        return Err(
            "Choose a plain file name without path separators or reserved characters".into(),
        );
    }
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "CLOCK$"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        return Err("Reserved system file name".into());
    }
    Ok(())
}

pub fn read_outgoing(path: &Path) -> Result<(String, Vec<u8>), String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or("File name is not valid UTF-8")?;
    valid_name(name)?;
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("Select a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_SIZE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_SIZE {
        return Err("Preview file limit is 16 MiB".into());
    }
    Ok((name.to_owned(), bytes))
}

pub fn save_received(directory: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    valid_name(name)?;
    if bytes.len() > MAX_FILE_SIZE {
        return Err("Preview file limit is 16 MiB".into());
    }
    fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let destination = directory
        .canonicalize()
        .map_err(|e| e.to_string())?
        .join(name);
    // create_new rejects an existing file or symlink; incoming content cannot overwrite it.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination)
        .map_err(|e| e.to_string())?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_paths_and_windows_devices() {
        for name in [
            "../secret",
            "..\\secret",
            "C:secret",
            "NUL",
            "con.txt",
            "LPT1.txt",
            "x.",
            "x\n",
        ] {
            assert!(valid_name(name).is_err(), "{name}");
        }
        assert!(valid_name("rapor.txt").is_ok());
    }
    #[test]
    fn receive_never_overwrites() {
        let dir = std::env::temp_dir().join(format!(
            "opendesk-transfer-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        let saved = save_received(&dir, "hello.txt", b"first").unwrap();
        assert_eq!(
            read_outgoing(&saved).unwrap(),
            ("hello.txt".into(), b"first".to_vec())
        );
        assert!(save_received(&dir, "hello.txt", b"second").is_err());
        assert_eq!(fs::read(&saved).unwrap(), b"first");
        fs::remove_file(saved).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
