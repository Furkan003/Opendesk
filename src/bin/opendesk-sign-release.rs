use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use rand::RngCore;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
fn private_path(path: &Path) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or("Private key path has no parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let work = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("work")
        .canonicalize()
        .map_err(|_| "Create a work directory for private signing keys first")?;
    if !parent.starts_with(work) {
        return Err("Private signing keys must stay inside this workspace's work directory".into());
    }
    Ok(parent.join(path.file_name().ok_or("Invalid key filename")?))
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str){
 Some("--keygen") if args.len()==3=>{let path=private_path(Path::new(&args[1]))?;let mut bytes=[0u8;32];rand::rngs::OsRng.fill_bytes(&mut bytes);let key=SigningKey::from_bytes(&bytes);let mut file=std::fs::OpenOptions::new().write(true).create_new(true).open(path).map_err(|e|e.to_string())?;file.write_all(bytes.iter().map(|b|format!("{b:02x}")).collect::<String>().as_bytes()).map_err(|e|e.to_string())?;std::fs::write(&args[2],key.verifying_key().to_bytes().iter().map(|b|format!("{b:02x}")).collect::<String>()).map_err(|e|e.to_string())?;println!("Signing key created; private material remains in work. Rebuild application after pinning the public key.");Ok(())},
 Some("--sign") if args.len()==4=>{let key=SigningKey::from_bytes(&opendesk::update::decode_hex::<32>(&std::fs::read_to_string(private_path(Path::new(&args[1]))?).map_err(|e|e.to_string())?)?);let payload=std::fs::read(&args[2]).map_err(|e|e.to_string())?;let _:opendesk::update::Manifest=serde_json::from_slice(&payload).map_err(|e|e.to_string())?;let envelope=opendesk::update::Envelope{payload:STANDARD.encode(&payload),signature:STANDARD.encode(key.sign(&payload).to_bytes())};std::fs::write(&args[3],serde_json::to_vec_pretty(&envelope).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;println!("Signed update manifest written.");Ok(())},
 _=>Err("Usage: opendesk-sign-release --keygen work/PRIVATE PUBLIC | --sign work/PRIVATE MANIFEST OUTPUT".into())}
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
