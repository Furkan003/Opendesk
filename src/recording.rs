//! Indexed Motion JPEG AVI, finalized on stop; frames are genuine JPEG video samples.
use std::{
    fs::{File, OpenOptions},
    io::{Seek, SeekFrom, Write},
    path::Path,
};
pub struct Recorder {
    started: std::time::Instant,
    fps: u32,
    file: File,
    index: Vec<(u32, u32)>,
    movi: u64,
    movi_size: u64,
    total: u64,
    length: u64,
    width: u32,
    height: u32,
}
fn u32w(f: &mut File, n: u32) -> std::io::Result<()> {
    f.write_all(&n.to_le_bytes())
}
fn chunk(f: &mut File, id: &[u8; 4], data: &[u8]) -> std::io::Result<()> {
    f.write_all(id)?;
    u32w(f, data.len() as u32)?;
    f.write_all(data)?;
    if data.len() % 2 == 1 {
        f.write_all(&[0])?;
    }
    Ok(())
}
fn words(values: &[u32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn patch(f: &mut File, pos: u64, value: u32) -> std::io::Result<()> {
    let end = f.stream_position()?;
    f.seek(SeekFrom::Start(pos))?;
    u32w(f, value)?;
    f.seek(SeekFrom::Start(end))?;
    Ok(())
}
impl Recorder {
    pub fn start(path: &Path, width: u32, height: u32, fps: u32) -> Result<Self, String> {
        Self::create(path, width, height, fps).map_err(|e| e.to_string())
    }
    fn create(path: &Path, width: u32, height: u32, fps: u32) -> std::io::Result<Self> {
        let mut f = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(path)?;
        f.write_all(b"RIFF")?;
        u32w(&mut f, 0)?;
        f.write_all(b"AVI LIST")?;
        let hdrl = f.stream_position()?;
        u32w(&mut f, 0)?;
        f.write_all(b"hdrl")?;
        let avih = f.stream_position()?;
        chunk(
            &mut f,
            b"avih",
            &words(&[
                1_000_000 / fps.max(1),
                0,
                0,
                0x10,
                0,
                0,
                1,
                0,
                width,
                height,
                0,
                0,
                0,
                0,
            ]),
        )?;
        f.write_all(b"LIST")?;
        let strl = f.stream_position()?;
        u32w(&mut f, 0)?;
        f.write_all(b"strl")?;
        let mut header = Vec::new();
        header.extend(b"vidsMJPG");
        header.extend(words(&[0, 0, 0, 1, fps.max(1), 0, 0, 0, u32::MAX, 0]));
        header.extend([0u8; 8]);
        let strh = f.stream_position()?;
        chunk(&mut f, b"strh", &header)?;
        let mut format = words(&[40, width, height]);
        format.extend(1u16.to_le_bytes());
        format.extend(24u16.to_le_bytes());
        format.extend(b"MJPG");
        format.extend(words(&[
            width.saturating_mul(height).saturating_mul(3),
            0,
            0,
            0,
            0,
        ]));
        chunk(&mut f, b"strf", &format)?;
        let end = f.stream_position()?;
        patch(&mut f, strl, (end - strl - 4) as u32)?;
        patch(&mut f, hdrl, (end - hdrl - 4) as u32)?;
        f.write_all(b"LIST")?;
        let movi_size = f.stream_position()?;
        u32w(&mut f, 0)?;
        let movi = f.stream_position()?;
        f.write_all(b"movi")?;
        Ok(Self {
            started: std::time::Instant::now(),
            fps: fps.max(1),
            file: f,
            index: Vec::new(),
            movi,
            movi_size,
            total: avih + 8 + 16,
            length: strh + 8 + 32,
            width,
            height,
        })
    }
    pub fn frame(&mut self, jpeg: &[u8], width: u32, height: u32) -> Result<(), String> {
        if (width, height) != (self.width, self.height) {
            return Err(
                "Display dimensions changed. Stop recording before switching displays.".into(),
            );
        }
        if self.file.stream_position().map_err(|e| e.to_string())? > 1_800_000_000 {
            return Err("AVI reached 1.8 GB recording limit".into());
        }
        let offset = self.file.stream_position().map_err(|e| e.to_string())? - self.movi;
        chunk(&mut self.file, b"00dc", jpeg).map_err(|e| e.to_string())?;
        self.index.push((offset as u32, jpeg.len() as u32));
        Ok(())
    }
    pub fn realtime_frame(&mut self, jpeg: &[u8], width: u32, height: u32) -> Result<(), String> {
        let expected =
            (self.started.elapsed().as_secs_f64() * f64::from(self.fps)).floor() as usize + 1;
        // Preserve wall-clock duration with repeated samples if network delivery slows.
        // Cap catch-up per arrival to prevent long pauses from blocking the UI.
        for _ in 0..expected.saturating_sub(self.index.len()).min(120) {
            self.frame(jpeg, width, height)?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(), String> {
        self.finalize().map_err(|e| e.to_string())
    }
    fn finalize(&mut self) -> std::io::Result<()> {
        if self.index.is_empty() {
            return self.file.flush();
        }
        let end = self.file.stream_position()?;
        patch(
            &mut self.file,
            self.movi_size,
            (end - self.movi_size - 4) as u32,
        )?;
        let mut index = Vec::with_capacity(self.index.len() * 16);
        for &(offset, size) in &self.index {
            index.extend(b"00dc");
            index.extend(words(&[0x10, offset, size]));
        }
        chunk(&mut self.file, b"idx1", &index)?;
        let count = self.index.len() as u32;
        let size = self.file.stream_position()?;
        patch(&mut self.file, 4, (size - 8) as u32)?;
        patch(&mut self.file, self.total, count)?;
        patch(&mut self.file, self.length, count)?;
        self.index.clear();
        self.file.flush()
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writes_indexed_avi() {
        let p = std::env::temp_dir().join(format!("opendesk-rec-{}.avi", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let mut r = Recorder::start(&p, 1, 1, 6).unwrap();
        r.frame(&[0xff, 0xd8, 0xff, 0xd9], 1, 1).unwrap();
        r.finish().unwrap();
        let b = std::fs::read(&p).unwrap();
        assert_eq!(&b[..4], b"RIFF");
        assert!(b.windows(4).any(|w| w == b"idx1"));
        assert_eq!(
            u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize,
            b.len() - 8
        );
        std::fs::remove_file(p).unwrap();
    }
}
