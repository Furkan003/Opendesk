//! Permission-gated PCM system audio; microphone capture is deliberately not used.
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver},
    },
};

#[derive(Clone, Debug)]
pub struct AudioPacket {
    pub sample_rate: u32,
    pub channels: u8,
    pub pcm: Vec<i16>,
}

pub struct AudioCapture {
    receiver: Receiver<AudioPacket>,
    errors: Receiver<String>,
    _stream: Option<cpal::Stream>,
    #[cfg(target_os = "linux")]
    child: Option<std::process::Child>,
}
impl AudioCapture {
    pub fn start() -> Result<Self, String> {
        #[cfg(target_os = "windows")]
        {
            let device = cpal::default_host()
                .default_output_device()
                .ok_or("No audio output device for WASAPI loopback")?;
            let supported = device.default_output_config().map_err(|e| e.to_string())?;
            let config: cpal::StreamConfig = supported.clone().into();
            let (tx, receiver) = mpsc::sync_channel(8);
            let (etx, errors) = mpsc::sync_channel(2);
            // CPAL's WASAPI backend adds AUDCLNT_STREAMFLAGS_LOOPBACK for an
            // input stream built on an output endpoint.
            let rate = config.sample_rate.0;
            let channels = config.channels as usize;
            let err = move |e| {
                let _ = etx.try_send(format!("System audio capture stopped: {e}"));
            };
            let stream = match supported.sample_format() {
                cpal::SampleFormat::F32 => {
                    let mut p = Packetizer::new(rate, channels, tx);
                    device.build_input_stream(
                        &config,
                        move |data: &[f32], _| p.feed(data.iter().copied()),
                        err,
                        None,
                    )
                }
                cpal::SampleFormat::I16 => {
                    let mut p = Packetizer::new(rate, channels, tx);
                    device.build_input_stream(
                        &config,
                        move |data: &[i16], _| p.feed(data.iter().map(|v| *v as f32 / 32768.0)),
                        err,
                        None,
                    )
                }
                cpal::SampleFormat::U16 => {
                    let mut p = Packetizer::new(rate, channels, tx);
                    device.build_input_stream(
                        &config,
                        move |data: &[u16], _| {
                            p.feed(data.iter().map(|v| (*v as f32 - 32768.0) / 32768.0))
                        },
                        err,
                        None,
                    )
                }
                other => return Err(format!("Unsupported system audio format: {other:?}")),
            }
            .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;
            Ok(Self {
                receiver,
                errors,
                _stream: Some(stream),
            })
        }
        #[cfg(target_os = "linux")]
        {
            use std::{
                io::Read,
                process::{Command, Stdio},
            };
            let output = Command::new("pactl")
                .arg("get-default-sink")
                .output()
                .map_err(|e| format!("System audio needs pactl (PulseAudio/PipeWire): {e}"))?;
            if !output.status.success() {
                return Err("Could not resolve the default PulseAudio/PipeWire output sink".into());
            }
            let sink = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if sink.is_empty() || sink.contains('\n') {
                return Err("Invalid default audio sink".into());
            }
            let mut child = Command::new("ffmpeg")
                .args([
                    "-nostdin",
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-f",
                    "pulse",
                    "-i",
                    &format!("{sink}.monitor"),
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                    "-f",
                    "s16le",
                    "pipe:1",
                ])
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| {
                    format!("System audio needs ffmpeg with PulseAudio input support: {e}")
                })?;
            let mut stdout = child.stdout.take().ok_or("Audio process has no output")?;
            let (tx, receiver) = mpsc::sync_channel(8);
            let (etx, errors) = mpsc::sync_channel(2);
            std::thread::spawn(move || {
                let mut bytes = [0u8; 7680];
                loop {
                    if let Err(e) = stdout.read_exact(&mut bytes) {
                        let _ = etx.try_send(format!("System audio process stopped: {e}"));
                        break;
                    }
                    let pcm = bytes
                        .chunks_exact(2)
                        .map(|b| i16::from_le_bytes([b[0], b[1]]))
                        .collect();
                    if let Err(mpsc::TrySendError::Disconnected(_)) = tx.try_send(AudioPacket {
                        sample_rate: 48000,
                        channels: 2,
                        pcm,
                    }) {
                        break;
                    }
                }
            });
            Ok(Self {
                receiver,
                errors,
                _stream: None,
                child: Some(child),
            })
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err("System audio capture is supported on Windows and Linux".into())
        }
    }
    pub fn try_recv(&self) -> Option<AudioPacket> {
        self.receiver.try_recv().ok()
    }
    pub fn try_error(&self) -> Option<String> {
        self.errors.try_recv().ok()
    }
}
#[cfg(target_os = "linux")]
impl Drop for AudioCapture {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(target_os = "windows")]
struct Packetizer {
    rate: u32,
    channels: usize,
    phase: u64,
    frame: Vec<f32>,
    pcm: Vec<i16>,
    tx: mpsc::SyncSender<AudioPacket>,
}
#[cfg(target_os = "windows")]
impl Packetizer {
    fn new(rate: u32, channels: usize, tx: mpsc::SyncSender<AudioPacket>) -> Self {
        Self {
            rate,
            channels,
            phase: 0,
            frame: Vec::with_capacity(channels),
            pcm: Vec::with_capacity(3840),
            tx,
        }
    }
    fn feed(&mut self, samples: impl Iterator<Item = f32>) {
        for sample in samples {
            self.frame.push(sample);
            if self.frame.len() != self.channels {
                continue;
            }
            self.phase += 48000;
            while self.phase >= self.rate as u64 {
                self.phase -= self.rate as u64;
                let l = self.frame[0];
                let r = *self.frame.get(1).unwrap_or(&l);
                self.pcm.extend([to_i16(l), to_i16(r)]);
                if self.pcm.len() >= 3840 {
                    let pcm = std::mem::replace(&mut self.pcm, Vec::with_capacity(3840));
                    let _ = self.tx.try_send(AudioPacket {
                        sample_rate: 48000,
                        channels: 2,
                        pcm,
                    });
                }
            }
            self.frame.clear();
        }
    }
}
#[cfg(target_os = "windows")]
fn to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0) as i16
}

pub struct AudioPlayer {
    queue: Arc<Mutex<VecDeque<f32>>>,
    _stream: cpal::Stream,
    rate: u32,
    channels: u16,
}
impl AudioPlayer {
    pub fn new() -> Result<Self, String> {
        let device = cpal::default_host()
            .default_output_device()
            .ok_or("No audio output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let config: cpal::StreamConfig = supported.clone().into();
        let queue = Arc::new(Mutex::new(VecDeque::<f32>::new()));
        let q = queue.clone();
        let err = |e| eprintln!("Opendesk audio output: {e}");
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _| {
                    let mut q = q.lock().unwrap_or_else(|e| e.into_inner());
                    for s in data {
                        *s = q.pop_front().unwrap_or(0.0);
                    }
                },
                err,
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _| {
                    let mut q = q.lock().unwrap_or_else(|e| e.into_inner());
                    for s in data {
                        *s = (q.pop_front().unwrap_or(0.0) * 32767.0) as i16;
                    }
                },
                err,
                None,
            ),
            cpal::SampleFormat::U16 => device.build_output_stream(
                &config,
                move |data: &mut [u16], _| {
                    let mut q = q.lock().unwrap_or_else(|e| e.into_inner());
                    for s in data {
                        *s = ((q.pop_front().unwrap_or(0.0) + 1.0) * 32767.5) as u16;
                    }
                },
                err,
                None,
            ),
            other => return Err(format!("Unsupported audio output format: {other:?}")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            queue,
            _stream: stream,
            rate: config.sample_rate.0,
            channels: config.channels,
        })
    }
    pub fn push(&mut self, packet: AudioPacket) -> Result<(), String> {
        if !(8000..=48000).contains(&packet.sample_rate)
            || !(1..=2).contains(&packet.channels)
            || packet.pcm.len() > 4096
            || !packet.pcm.len().is_multiple_of(packet.channels as usize)
        {
            return Err("Invalid audio packet".into());
        }
        let source_frames = packet.pcm.len() / packet.channels as usize;
        if source_frames == 0 {
            return Ok(());
        }
        let frames = (source_frames as u64 * self.rate as u64 / packet.sample_rate as u64) as usize;
        let mut q = self.queue.lock().map_err(|_| "Audio queue unavailable")?;
        // Keep latency bounded to 200ms even if network delivery arrives in bursts.
        let max = (self.rate as usize * self.channels as usize) / 5;
        for i in 0..frames {
            let src = ((i as u64 * packet.sample_rate as u64 / self.rate as u64) as usize)
                .min(source_frames - 1);
            for ch in 0..self.channels as usize {
                let source_ch = if packet.channels == 1 { 0 } else { ch.min(1) };
                q.push_back(
                    packet.pcm[src * packet.channels as usize + source_ch] as f32 / 32768.0,
                );
            }
        }
        while q.len() > max {
            q.pop_front();
        }
        Ok(())
    }
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;
    #[test]
    fn loopback_downsampling_emits_bounded_stereo_packets() {
        let (tx, rx) = mpsc::sync_channel(8);
        let mut capture = Packetizer::new(96000, 2, tx);
        capture.feed(vec![0.5f32; 7680].into_iter());
        let packet = rx.try_recv().unwrap();
        assert_eq!(packet.sample_rate, 48000);
        assert_eq!(packet.channels, 2);
        assert_eq!(packet.pcm.len(), 3840);
        assert!(packet.pcm.iter().all(|v| *v == 16383));
        assert!(rx.try_recv().is_err());
    }
}
