//! Explicit diagnostic only: auto-approves a view-only session containing synthetic data.
use opendesk::network::{self, Command, Event};
use std::{
    io::Write,
    time::{Duration, Instant},
};
fn main() {
    if let Err(error) = run() {
        eprintln!("PROBE FAILED: {error}");
        std::process::exit(1)
    }
}
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4
        || !matches!(
            args[1].as_str(),
            "--host"
                | "--connect"
                | "--host-relay"
                | "--connect-relay"
                | "--host-p2p"
                | "--connect-p2p"
        )
    {
        return Err("Usage: opendesk-probe --host|--connect|--host-relay|--connect-relay|--host-p2p|--connect-p2p ADDRESS EPHEMERAL_CODE (synthetic diagnostics; host auto-approves view-only; p2p ignores ADDRESS)".into());
    }
    let host = args[1].starts_with("--host");
    let p2p = args[1].ends_with("-p2p");
    let connection = if args[1] == "--host-p2p" {
        opendesk::p2p::host(&args[3])
    } else if args[1] == "--connect-p2p" {
        opendesk::p2p::connect(&args[3])
    } else if args[1] == "--host-relay" {
        network::host_relay(&args[2], &args[3])
    } else if args[1] == "--connect-relay" {
        network::connect_relay(&args[2], &args[3])
    } else if host {
        network::host(&args[2], &args[3])
    } else {
        network::connect(&args[2], &args[3])
    }?;
    println!(
        "PROBE {} {} {}",
        std::env::consts::OS,
        if host { "HOST" } else { "VIEWER" },
        args[2]
    );
    std::io::stdout().flush().map_err(|e| e.to_string())?;
    // Internet discovery and hole punching take longer than a LAN connect.
    let deadline = Instant::now() + Duration::from_secs(if p2p { 45 } else { 15 });
    let mut chat = false;
    let mut file = false;
    let mut frame = host;
    let mut ack = false;
    let mut stop = false;
    loop {
        let event = connection
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| e.to_string())?;
        match event {
            Event::Status(message) => {
                println!("STATUS {message}");
                std::io::stdout().flush().map_err(|e| e.to_string())?;
            }
            Event::Incoming { .. } if host => connection
                .commands
                .send(Command::Accept { control: false })
                .map_err(|e| e.to_string())?,
            Event::Connected { control: false, .. } => {
                if host {
                    let pixels = image::RgbImage::from_pixel(2, 2, image::Rgb([210, 45, 75]));
                    let mut jpeg = Vec::new();
                    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 100)
                        .encode_image(&pixels)
                        .map_err(|e| e.to_string())?;
                    connection
                        .commands
                        .send(Command::Frame(jpeg))
                        .map_err(|e| e.to_string())?;
                }
                connection
                    .commands
                    .send(Command::Chat(
                        if host { "host-v2" } else { "viewer-v2" }.into(),
                    ))
                    .map_err(|e| e.to_string())?;
                connection
                    .commands
                    .send(Command::SendFile {
                        name: if host { "host.bin" } else { "viewer.bin" }.into(),
                        bytes: (0..=255).collect(),
                    })
                    .map_err(|e| e.to_string())?;
            }
            Event::Connected { control: true, .. } => {
                return Err("Diagnostic unexpectedly enabled input".into());
            }
            Event::Chat(text) if text == if host { "viewer-v2" } else { "host-v2" } => chat = true,
            Event::Chat(text) if host && text == "ACK" => ack = true,
            Event::IncomingFile { name, bytes } => {
                if name != if host { "viewer.bin" } else { "host.bin" }
                    || bytes != (0..=255).collect::<Vec<u8>>()
                {
                    return Err("Cross-OS file payload mismatch".into());
                }
                file = true;
            }
            Event::Frame { jpeg } if !host => {
                let pixels = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
                    .map_err(|e| e.to_string())?
                    .to_rgb8();
                if pixels.dimensions() != (2, 2)
                    || pixels.pixels().any(|p| {
                        p.0.iter()
                            .zip([210u8, 45, 75])
                            .any(|(v, w)| v.abs_diff(w) > 10)
                    })
                {
                    return Err("Cross-OS JPEG decoding mismatch".into());
                }
                frame = true;
            }
            Event::Disconnected => {
                if !chat || !file || !frame || host && !ack {
                    return Err("Disconnected before diagnostic exchange completed".into());
                }
                println!(
                    "PASS os={} role={} jpeg_decoded={} chat=true file=true input=false",
                    std::env::consts::OS,
                    if host { "host" } else { "viewer" },
                    !host
                );
                return Ok(());
            }
            Event::Input(_) => return Err("Unexpected remote input in view-only diagnostic".into()),
            _ => {}
        }
        if !host && chat && file && frame && !ack {
            connection
                .commands
                .send(Command::Chat("ACK".into()))
                .map_err(|e| e.to_string())?;
            ack = true;
        }
        if host && chat && file && ack && !stop {
            connection
                .commands
                .send(Command::Stop)
                .map_err(|e| e.to_string())?;
            stop = true;
        }
    }
}
