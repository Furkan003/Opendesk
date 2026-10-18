//! Signed, explicitly applied updates. No hosted endpoint is configured by default.
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
const MAX_ASSET: u64 = 512 * 1024 * 1024;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub version: String,
    pub platform: String,
    pub asset_url: String,
    pub sha256: String,
    pub size: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub payload: String,
    pub signature: String,
}
#[derive(Clone)]
pub struct Available {
    pub manifest: Manifest,
    envelope: Vec<u8>,
}
pub fn platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}
pub fn decode_hex<const N: usize>(text: &str) -> Result<[u8; N], String> {
    let text = text.trim();
    if text.len() != N * 2 || !text.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid hex key or digest".into());
    }
    let mut bytes = [0; N];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).map_err(|e| e.to_string())?;
    }
    Ok(bytes)
}
pub fn validate_url(address: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(address).map_err(|e| e.to_string())?;
    let local = matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err("Updates require HTTPS (HTTP permitted only on loopback for testing)".into());
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err("Update URL cannot contain credentials or fragments".into());
    }
    Ok(url)
}
fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(120))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())
}
fn verify_with(bytes: &[u8], key: &[u8; 32]) -> Result<Manifest, String> {
    if bytes.len() > 32768 {
        return Err("Update manifest exceeds size limit".into());
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let payload = STANDARD
        .decode(&envelope.payload)
        .map_err(|e| e.to_string())?;
    let signature = STANDARD
        .decode(&envelope.signature)
        .map_err(|e| e.to_string())?;
    VerifyingKey::from_bytes(key)
        .map_err(|e| e.to_string())?
        .verify_strict(
            &payload,
            &Signature::from_slice(&signature).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "Update signature is invalid".to_string())?;
    let manifest: Manifest = serde_json::from_slice(&payload).map_err(|e| e.to_string())?;
    validate_url(&manifest.asset_url)?;
    decode_hex::<32>(&manifest.sha256)?;
    if manifest.platform != platform() {
        return Err("Update targets a different platform".into());
    }
    if manifest.size == 0 || manifest.size > MAX_ASSET {
        return Err("Update asset size is invalid".into());
    }
    let target = semver::Version::parse(&manifest.version).map_err(|e| e.to_string())?;
    let current = semver::Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
    if target <= current {
        return Err("No newer update is available (downgrades are rejected)".into());
    }
    Ok(manifest)
}
pub fn verify(bytes: &[u8]) -> Result<Manifest, String> {
    verify_with(
        bytes,
        &decode_hex::<32>(include_str!("../assets/update-public-key.txt"))?,
    )
}
pub fn check(endpoint: &str) -> Result<Available, String> {
    let url = validate_url(endpoint)?;
    let mut response = client()?
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    response
        .by_ref()
        .take(32769)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    let manifest = verify(&bytes)?;
    Ok(Available {
        manifest,
        envelope: bytes,
    })
}
fn hash_file(path: &Path) -> Result<(u64, String), String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut size = 0;
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        size += n as u64;
        hash.update(&buffer[..n]);
    }
    Ok((size, format!("{:x}", hash.finalize())))
}
pub fn verify_asset(path: &Path, manifest: &Manifest) -> Result<(), String> {
    let (size, hash) = hash_file(path)?;
    if size != manifest.size || hash != manifest.sha256.to_ascii_lowercase() {
        return Err("Downloaded update size or SHA-256 does not match signed manifest".into());
    }
    Ok(())
}
pub fn download(available: &Available) -> Result<PathBuf, String> {
    let manifest = verify(&available.envelope)?;
    let directory =
        std::env::temp_dir().join(format!("Opendesk-update-{:016x}", rand::random::<u64>()));
    std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
    let path = directory.join("payload");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    let mut response = client()?
        .get(validate_url(&manifest.asset_url)?)
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|e| e.to_string())?;
    let count = std::io::copy(&mut response.by_ref().take(manifest.size + 1), &mut file)
        .map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    drop(file);
    if count != manifest.size {
        let _ = std::fs::remove_file(&path);
        return Err("Update download size mismatch".into());
    }
    verify_asset(&path, &manifest)?;
    std::fs::write(directory.join("manifest.json"), &available.envelope)
        .map_err(|e| e.to_string())?;
    Ok(directory)
}
pub fn verify_staged(directory: &Path) -> Result<PathBuf, String> {
    let bytes = std::fs::read(directory.join("manifest.json")).map_err(|e| e.to_string())?;
    let manifest = verify(&bytes)?;
    let payload = directory.join("payload");
    verify_asset(&payload, &manifest)?;
    Ok(payload)
}
pub fn apply(directory: &Path) -> Result<(), String> {
    verify_staged(directory)?;
    let parent = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .parent()
        .ok_or("Application path has no parent")?
        .to_path_buf();
    let helper = parent.join(if cfg!(windows) {
        "opendesk-updater.exe"
    } else {
        "opendesk-updater"
    });
    if !helper.is_file() {
        return Err("Updater helper missing; install the full bundle first".into());
    }
    let mut command = std::process::Command::new(helper);
    command.arg("--apply").arg(directory);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command.spawn().map_err(|e| e.to_string())?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    fn signed(manifest: &Manifest, key: &SigningKey) -> Vec<u8> {
        let payload = serde_json::to_vec(manifest).unwrap();
        serde_json::to_vec(&Envelope {
            payload: STANDARD.encode(&payload),
            signature: STANDARD.encode(key.sign(&payload).to_bytes()),
        })
        .unwrap()
    }
    fn manifest() -> Manifest {
        Manifest {
            version: "999.0.0".into(),
            platform: platform(),
            asset_url: "https://example.org/opendesk".into(),
            sha256: "00".repeat(32),
            size: 10,
        }
    }
    #[test]
    fn validates_signature_and_platform() {
        let key = SigningKey::from_bytes(&[42; 32]);
        let mut m = manifest();
        assert!(verify_with(&signed(&m, &key), &key.verifying_key().to_bytes()).is_ok());
        m.platform = "wrong".into();
        assert!(verify_with(&signed(&m, &key), &key.verifying_key().to_bytes()).is_err());
        assert!(
            verify_with(
                &signed(&manifest(), &key),
                &SigningKey::from_bytes(&[43; 32]).verifying_key().to_bytes()
            )
            .is_err()
        );
    }
    #[test]
    fn rejects_downgrade_and_insecure_url() {
        let key = SigningKey::from_bytes(&[42; 32]);
        let mut m = manifest();
        m.version = "0.0.1".into();
        assert!(verify_with(&signed(&m, &key), &key.verifying_key().to_bytes()).is_err());
        assert!(validate_url("http://example.com/update").is_err());
        assert!(validate_url("http://127.0.0.1:8080/update").is_ok());
        assert!(validate_url("https://user:pass@example.org").is_err());
    }
    #[test]
    fn rejects_wrong_asset_checksum() {
        let path = std::env::temp_dir().join(format!("Opendesk-hash-{}", rand::random::<u64>()));
        std::fs::write(&path, b"test").unwrap();
        assert!(verify_asset(&path, &manifest()).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
