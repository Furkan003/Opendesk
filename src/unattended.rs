//! Explicit opt-in password + TOTP access. Secret material lives in the OS credential store.
use crate::network::Permissions;
use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString},
};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;

#[derive(Serialize, Deserialize)]
struct Stored {
    enabled: bool,
    password_hash: String,
    permissions: Permissions,
    last_step: Option<u64>,
    failures: u32,
    blocked_until: u64,
}
pub struct Policy {
    stored: Stored,
}
pub struct Enrollment {
    pub secret_base32: String,
    pub otpauth_uri: String,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn config_dir() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let directory = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let directory = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    directory
        .map(|d| d.join("Opendesk"))
        .ok_or("Cannot locate user configuration directory".into())
}
fn path() -> Result<PathBuf, String> {
    Ok(config_dir()?.join("unattended.json"))
}
fn credential(name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("dev.opendesk", name).map_err(|e| format!("OS credential store: {e}"))
}
fn save(stored: &Stored) -> Result<(), String> {
    let path = path()?;
    let parent = path.parent().ok_or("Invalid configuration path")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", rand::random::<u64>()));
    fs::write(
        &temp,
        serde_json::to_vec_pretty(stored).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())?;
    }
    // The replacement is local policy metadata, never remote file content.
    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, path).map_err(|e| e.to_string())
}
fn read() -> Result<Option<Stored>, String> {
    match fs::read(path()?) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn begin_enrollment(
    password: &str,
    pairing_code: &str,
    permissions: Permissions,
) -> Result<Enrollment, String> {
    if password.len() < 12 || password.len() > 1024 {
        return Err("Unattended password must contain 12–1024 bytes".into());
    }
    // Validate pairing input without opening a connection.
    crate::invite::encode("localhost:7878", false, pairing_code)?;
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| e.to_string())?
        .to_string();
    let mut secret = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let encoded = base32_encode(&secret);
    credential("unattended-totp")?
        .set_password(&encoded)
        .map_err(|e| format!("Cannot save TOTP secret: {e}"))?;
    credential("unattended-pairing")?
        .set_password(pairing_code)
        .map_err(|e| format!("Cannot save pairing key: {e}"))?;
    save(&Stored {
        enabled: false,
        password_hash: hash,
        permissions,
        last_step: None,
        failures: 0,
        blocked_until: 0,
    })?;
    Ok(Enrollment {
        otpauth_uri: format!(
            "otpauth://totp/Opendesk:unattended?secret={encoded}&issuer=Opendesk&algorithm=SHA1&digits=6&period=30"
        ),
        secret_base32: encoded,
    })
}
pub fn confirm_enrollment(token: &str) -> Result<Policy, String> {
    let mut stored = read()?.ok_or("Begin enrollment first")?;
    if stored.enabled {
        return Err("Unattended access is already enabled".into());
    }
    let secret = load_totp()?;
    let step =
        matching_step(&secret, token, now() / 30).ok_or("Authenticator code did not match")?;
    stored.enabled = true;
    stored.last_step = Some(step);
    save(&stored)?;
    Ok(Policy { stored })
}
pub fn load_pairing_code() -> Result<String, String> {
    credential("unattended-pairing")?
        .get_password()
        .map_err(|e| format!("Cannot load stored pairing key: {e}"))
}
fn load_totp() -> Result<Vec<u8>, String> {
    base32_decode(
        &credential("unattended-totp")?
            .get_password()
            .map_err(|e| e.to_string())?,
    )
}
pub fn disable() -> Result<(), String> {
    if let Some(mut stored) = read()? {
        stored.enabled = false;
        save(&stored)?;
    }
    for name in ["unattended-totp", "unattended-pairing"] {
        if let Ok(entry) = credential(name) {
            let _ = entry.delete_credential();
        }
    }
    Ok(())
}
impl Policy {
    pub fn load() -> Result<Option<Self>, String> {
        Ok(read()?.filter(|s| s.enabled).map(|stored| Self { stored }))
    }
    pub fn verify(&mut self, password: &str, token: &str) -> Result<Permissions, String> {
        let time = now();
        if !self.stored.enabled || time < self.stored.blocked_until {
            return Err("Unattended authentication is disabled or temporarily locked".into());
        }
        let parsed = PasswordHash::new(&self.stored.password_hash).map_err(|e| e.to_string())?;
        let password_ok = password.len() <= 1024
            && Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok();
        let step = matching_step(&load_totp()?, token, time / 30);
        let fresh = step.is_some_and(|s| self.stored.last_step.is_none_or(|last| s > last));
        if !password_ok || !fresh {
            self.stored.failures = self.stored.failures.saturating_add(1);
            if self.stored.failures >= 5 {
                self.stored.blocked_until = time + 60;
                self.stored.failures = 0;
            }
            save(&self.stored)?;
            return Err("Password or authenticator code rejected".into());
        }
        self.stored.last_step = step;
        self.stored.failures = 0;
        self.stored.blocked_until = 0;
        save(&self.stored)?;
        Ok(self.stored.permissions)
    }
}
fn token_at(secret: &[u8], step: u64) -> String {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC accepts all key lengths");
    mac.update(&step.to_be_bytes());
    let digest = mac.finalize().into_bytes();
    let offset = (digest[19] & 15) as usize;
    let value = u32::from_be_bytes(digest[offset..offset + 4].try_into().unwrap()) & 0x7fff_ffff;
    format!("{:06}", value % 1_000_000)
}
fn matching_step(secret: &[u8], token: &str, current: u64) -> Option<u64> {
    if token.len() != 6 || !token.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    [
        current.checked_sub(1),
        Some(current),
        current.checked_add(1),
    ]
    .into_iter()
    .flatten()
    .find(|step| bool::from(token_at(secret, *step).as_bytes().ct_eq(token.as_bytes())))
}
fn base32_encode(bytes: &[u8]) -> String {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let (mut bits, mut count) = (0u32, 0u8);
    for byte in bytes {
        bits = (bits << 8) | *byte as u32;
        count += 8;
        while count >= 5 {
            count -= 5;
            out.push(alphabet[((bits >> count) & 31) as usize] as char);
        }
    }
    if count > 0 {
        out.push(alphabet[((bits << (5 - count)) & 31) as usize] as char);
    }
    out
}
fn base32_decode(text: &str) -> Result<Vec<u8>, String> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let (mut bits, mut count) = (0u32, 0u8);
    let mut out = Vec::new();
    for c in text.bytes() {
        let n = alphabet
            .iter()
            .position(|b| *b == c)
            .ok_or("Invalid stored TOTP secret")?;
        bits = (bits << 5) | n as u32;
        count += 5;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Ok(out)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rfc4226_totp_and_base32_vectors() {
        let secret = b"12345678901234567890";
        assert_eq!(token_at(secret, 0), "755224");
        assert_eq!(token_at(secret, 1), "287082");
        assert_eq!(base32_decode(&base32_encode(secret)).unwrap(), secret);
        assert_eq!(matching_step(secret, "287082", 1), Some(1));
        assert!(matching_step(secret, "12345", 1).is_none());
    }
}
