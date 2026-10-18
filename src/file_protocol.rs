//! Bounded-memory, consent-driven folder transfer with authenticated resume prefixes.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
pub const MAX_ENTRIES: usize = 128;
pub const MAX_TOTAL: u64 = 1024 * 1024 * 1024 * 1024;
pub type FileChunk = (usize, u64, Vec<u8>, bool);
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub relative: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub entries: Vec<Entry>,
    pub total: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResumeRequest {
    pub index: usize,
    pub offset: u64,
    pub prefix_hash: String,
}
fn hex(bytes: impl AsRef<[u8]>) -> String {
    bytes.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}
fn hash_file(path: &Path, limit: u64) -> Result<String, String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut left = limit;
    let mut b = [0u8; 65536];
    while left > 0 {
        let n = f
            .read(&mut b[..left.min(65536) as usize])
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("File changed or partial data missing".into());
        }
        h.update(&b[..n]);
        left -= n as u64;
    }
    Ok(hex(h.finalize()))
}
fn plain_component(component: &str) -> bool {
    if component.is_empty()
        || component == "."
        || component == ".."
        || component.ends_with(['.', ' '])
        || component
            .chars()
            .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
    {
        return false;
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    !["CON", "PRN", "AUX", "NUL", "CLOCK$"].contains(&stem.as_str())
        && !(stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}
fn valid_path(relative: &str) -> bool {
    !relative.is_empty()
        && relative.len() <= 240
        && !relative.starts_with('/')
        && relative.split('/').all(plain_component)
        && !relative
            .split('/')
            .any(|part| part.eq_ignore_ascii_case(".opendesk-partials"))
}
fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    meta.file_type().is_symlink()
}
fn manifest_id(entries: &[Entry]) -> Result<String, String> {
    Ok(hex(Sha256::digest(
        serde_json::to_vec(entries).map_err(|e| e.to_string())?,
    )))
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.entries.is_empty()
            || self.entries.len() > MAX_ENTRIES
            || self.total > MAX_TOTAL
            || self.id != manifest_id(&self.entries)?
        {
            return Err("Invalid transfer manifest".into());
        }
        let mut seen = std::collections::HashSet::new();
        let mut total = 0u64;
        for e in &self.entries {
            if !valid_path(&e.relative)
                || e.sha256.len() != 64
                || !e.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || !seen.insert(e.relative.to_ascii_lowercase())
            {
                return Err("Unsafe or duplicate transfer path".into());
            }
            total = total.checked_add(e.size).ok_or("Transfer size overflow")?;
        }
        if total != self.total {
            return Err("Transfer size mismatch".into());
        }
        Ok(())
    }
}
pub struct Sender {
    pub manifest: Manifest,
    paths: Vec<PathBuf>,
    active: Option<(usize, File, u64)>,
}
impl Sender {
    pub fn open(path: &Path) -> Result<Self, String> {
        let root = path.canonicalize().map_err(|e| e.to_string())?;
        if is_link(&fs::symlink_metadata(path).map_err(|e| e.to_string())?) {
            return Err("Symlink transfers are not allowed".into());
        }
        let base = root.parent().ok_or("Missing parent directory")?;
        let mut paths = Vec::new();
        collect(&root, &mut paths)?;
        paths.sort();
        if paths.is_empty() {
            return Err("Folder contains no regular files".into());
        }
        let mut entries = Vec::new();
        let mut total = 0u64;
        for p in &paths {
            let relative = p
                .strip_prefix(base)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or("Non-UTF8 file name")?
                .replace('\\', "/");
            if !valid_path(&relative) {
                return Err("Unsupported file or folder name".into());
            }
            let size = p.metadata().map_err(|e| e.to_string())?.len();
            total = total.checked_add(size).ok_or("Transfer size overflow")?;
            if total > MAX_TOTAL {
                return Err("Transfer exceeds 1 TiB".into());
            }
            entries.push(Entry {
                relative,
                size,
                sha256: hash_file(p, size)?,
            });
        }
        let manifest = Manifest {
            id: manifest_id(&entries)?,
            entries,
            total,
        };
        manifest.validate()?;
        Ok(Self {
            manifest,
            paths,
            active: None,
        })
    }
    pub fn resume(&mut self, request: &ResumeRequest) -> Result<(), String> {
        let entry = self
            .manifest
            .entries
            .get(request.index)
            .ok_or("Invalid file index")?;
        if request.offset > entry.size
            || request.prefix_hash != hash_file(&self.paths[request.index], request.offset)?
        {
            return Err(
                "Resume prefix does not match source; discard the partial transfer and retry"
                    .into(),
            );
        }
        let mut file = File::open(&self.paths[request.index]).map_err(|e| e.to_string())?;
        if file.metadata().map_err(|e| e.to_string())?.len() != entry.size {
            return Err("Source file changed".into());
        }
        file.seek(SeekFrom::Start(request.offset))
            .map_err(|e| e.to_string())?;
        self.active = Some((request.index, file, request.offset));
        Ok(())
    }
    pub fn next_chunk(&mut self, max: usize) -> Result<Option<FileChunk>, String> {
        let Some((index, file, offset)) = self.active.as_mut() else {
            return Ok(None);
        };
        let size = self.manifest.entries[*index].size;
        let start = *offset;
        let mut bytes = vec![0; (size - *offset).min(max as u64) as usize];
        file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
        *offset += bytes.len() as u64;
        let finished = *offset == size;
        let index = *index;
        if finished {
            self.active = None;
        }
        Ok(Some((index, start, bytes, finished)))
    }
}
fn collect(path: &Path, result: &mut Vec<PathBuf>) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if is_link(&meta) {
        return Err("Folder contains a symlink".into());
    }
    if meta.is_file() {
        result.push(path.to_owned());
        if result.len() > MAX_ENTRIES {
            return Err("Folder exceeds 128 files".into());
        }
    } else if meta.is_dir() {
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            collect(&entry.map_err(|e| e.to_string())?.path(), result)?;
        }
    } else {
        return Err("Only regular files and folders can be sent".into());
    }
    Ok(())
}
pub struct Receiver {
    pub manifest: Manifest,
    root: PathBuf,
    staging: PathBuf,
    offsets: Vec<u64>,
    completed: Vec<bool>,
}
impl Receiver {
    pub fn open(root: &Path, manifest: Manifest) -> Result<Self, String> {
        manifest.validate()?;
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let staging = safe_directory(&root, &format!(".opendesk-partials/{}", manifest.id))?;
        let mut receiver = Self {
            offsets: vec![0; manifest.entries.len()],
            completed: vec![false; manifest.entries.len()],
            manifest,
            root,
            staging,
        };
        for i in 0..receiver.manifest.entries.len() {
            let entry = &receiver.manifest.entries[i];
            let dest = receiver.destination(i)?;
            if dest.exists() {
                if is_link(&fs::symlink_metadata(&dest).map_err(|e| e.to_string())?)
                    || !dest.is_file()
                    || dest.metadata().map_err(|e| e.to_string())?.len() != entry.size
                    || hash_file(&dest, entry.size)? != entry.sha256
                {
                    return Err(
                        "Destination exists with different contents; choose another folder".into(),
                    );
                }
                receiver.completed[i] = true;
                receiver.offsets[i] = entry.size;
                continue;
            }
            let part = receiver.partial(i);
            if part.exists() {
                let meta = fs::symlink_metadata(&part).map_err(|e| e.to_string())?;
                if !meta.is_file() || is_link(&meta) || meta.len() > entry.size {
                    return Err("Invalid partial transfer file".into());
                }
                receiver.offsets[i] = meta.len();
            } else {
                OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&part)
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(receiver)
    }
    fn partial(&self, index: usize) -> PathBuf {
        self.staging.join(format!("{index}.part"))
    }
    fn destination(&self, index: usize) -> Result<PathBuf, String> {
        let relative = &self.manifest.entries[index].relative;
        let (parent, name) = relative.rsplit_once('/').unwrap_or(("", relative));
        Ok(safe_directory(&self.root, parent)?.join(name))
    }
    pub fn requests(&self) -> Result<Vec<ResumeRequest>, String> {
        (0..self.manifest.entries.len())
            .filter(|i| !self.completed[*i])
            .map(|index| {
                Ok(ResumeRequest {
                    index,
                    offset: self.offsets[index],
                    prefix_hash: hash_file(&self.partial(index), self.offsets[index])?,
                })
            })
            .collect()
    }
    pub fn write_chunk(
        &mut self,
        index: usize,
        offset: u64,
        bytes: &[u8],
        finished: bool,
    ) -> Result<(), String> {
        let entry = self
            .manifest
            .entries
            .get(index)
            .ok_or("Invalid receive index")?;
        if self.completed[index]
            || offset != self.offsets[index]
            || offset
                .checked_add(bytes.len() as u64)
                .ok_or("Offset overflow")?
                > entry.size
            || finished != (offset + bytes.len() as u64 == entry.size)
        {
            return Err("Invalid transfer chunk offset or completion".into());
        }
        let part = self.partial(index);
        let meta = fs::symlink_metadata(&part).map_err(|e| e.to_string())?;
        if !meta.is_file() || is_link(&meta) || meta.len() != offset {
            return Err("Partial file changed".into());
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&part)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes).map_err(|e| e.to_string())?;
        self.offsets[index] += bytes.len() as u64;
        if finished {
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            if hash_file(&part, entry.size)? != entry.sha256 {
                return Err("Received file checksum mismatch".into());
            }
            fs::hard_link(&part, self.destination(index)?).map_err(|e| e.to_string())?;
            fs::remove_file(part).map_err(|e| e.to_string())?;
            self.completed[index] = true;
        }
        Ok(())
    }
    pub fn progress(&self) -> u64 {
        self.offsets.iter().sum()
    }
    pub fn is_complete(&self) -> bool {
        self.completed.iter().all(|done| *done)
    }
}
fn safe_directory(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let mut path = root.to_owned();
    if relative.is_empty() {
        return Ok(path);
    }
    for component in relative.split('/') {
        if !plain_component(component) {
            return Err("Unsafe directory name".into());
        }
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) => {
                if is_link(&meta) || !meta.is_dir() {
                    return Err("Destination parent is not a safe directory".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(|e| e.to_string())?
            }
            Err(e) => return Err(e.to_string()),
        }
        if !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(root)
        {
            return Err("Destination escapes selected directory".into());
        }
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn temp() -> PathBuf {
        let p = std::env::temp_dir().join(format!("opendesk-stream-{}", rand::random::<u64>()));
        fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn folder_transfer_resumes_and_never_overwrites() {
        let work = temp();
        let source = work.join("folder");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("one.bin"), vec![123; 100_000]).unwrap();
        fs::write(source.join("empty"), []).unwrap();
        let mut sender = Sender::open(&source).unwrap();
        let manifest = sender.manifest.clone();
        let output = work.join("received");
        let mut receiver = Receiver::open(&output, manifest.clone()).unwrap();
        let request = receiver
            .requests()
            .unwrap()
            .into_iter()
            .find(|r| sender.manifest.entries[r.index].size > 0)
            .unwrap();
        sender.resume(&request).unwrap();
        let (i, o, b, f) = sender.next_chunk(4096).unwrap().unwrap();
        receiver.write_chunk(i, o, &b, f).unwrap();
        drop(receiver);
        let mut receiver = Receiver::open(&output, manifest).unwrap();
        for request in receiver.requests().unwrap() {
            sender.resume(&request).unwrap();
            while let Some((i, o, b, f)) = sender.next_chunk(4096).unwrap() {
                receiver.write_chunk(i, o, &b, f).unwrap();
            }
        }
        assert!(receiver.is_complete());
        assert_eq!(
            fs::read(output.join("folder/one.bin")).unwrap(),
            vec![123; 100_000]
        );
        fs::write(output.join("folder/one.bin"), b"changed").unwrap();
        assert!(Receiver::open(&output, sender.manifest.clone()).is_err());
        fs::remove_dir_all(work).unwrap();
    }
    #[test]
    fn rejects_traversal_and_mismatched_prefix() {
        let work = temp();
        let file = work.join("safe");
        fs::write(&file, b"abc").unwrap();
        let mut sender = Sender::open(&file).unwrap();
        assert!(
            sender
                .resume(&ResumeRequest {
                    index: 0,
                    offset: 1,
                    prefix_hash: "wrong".into()
                })
                .is_err()
        );
        let mut manifest = sender.manifest.clone();
        manifest.entries[0].relative = "../escape".into();
        manifest.id = manifest_id(&manifest.entries).unwrap();
        assert!(manifest.validate().is_err());
        fs::remove_dir_all(work).unwrap();
    }
}
