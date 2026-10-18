//! Rendezvous byte relay. Desktop traffic remains Noise encrypted end-to-end.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub fn channel_id(key: &[u8; 32]) -> String {
    Sha256::digest(key)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Pending {
    stream: TcpStream,
    since: Instant,
}
type Hosts = Arc<Mutex<HashMap<String, Pending>>>;

/// Runs a bounded public TCP relay. Operators provide the listening socket.
/// Stop closes the listener and pending hosts; active bridges end on disconnect/timeout.
pub fn serve(listener: TcpListener, stop: Arc<AtomicBool>) -> io::Result<()> {
    listener.set_nonblocking(true)?;
    let hosts: Hosts = Arc::new(Mutex::new(HashMap::new()));
    let workers = Arc::new(AtomicUsize::new(0));
    while !stop.load(Ordering::Relaxed) {
        hosts
            .lock()
            .unwrap()
            .retain(|_, pending| pending.since.elapsed() < Duration::from_secs(60));
        match listener.accept() {
            Ok((stream, _)) => {
                if workers.load(Ordering::Relaxed) >= 64 {
                    continue;
                }
                workers.fetch_add(1, Ordering::Relaxed);
                let hosts = hosts.clone();
                let workers = workers.clone();
                thread::spawn(move || {
                    let _ = register(stream, &hosts);
                    workers.fetch_sub(1, Ordering::Relaxed);
                });
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20))
            }
            Err(e) => return Err(e),
        }
    }
    hosts.lock().unwrap().clear();
    Ok(())
}

fn register(mut stream: TcpStream, hosts: &Hosts) -> io::Result<()> {
    stream.set_nonblocking(false)?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut header = [0u8; 71];
    stream.read_exact(&mut header)?;
    if &header[..6] != b"ODESK1" || !header[7..].iter().all(|b| b.is_ascii_hexdigit()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid relay request",
        ));
    }
    let channel = String::from_utf8(header[7..].to_vec())
        .unwrap()
        .to_ascii_lowercase();
    match header[6] {
        b'H' => {
            let mut pending = hosts.lock().unwrap();
            if pending.len() >= 256 || pending.contains_key(&channel) {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "Relay channel unavailable",
                ));
            }
            stream.write_all(b"WAIT!\n")?;
            pending.insert(
                channel,
                Pending {
                    stream,
                    since: Instant::now(),
                },
            );
            Ok(())
        }
        b'V' => {
            let mut host = hosts
                .lock()
                .unwrap()
                .remove(&channel)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Host is not waiting"))?
                .stream;
            host.write_all(b"READY\n")?;
            stream.write_all(b"READY\n")?;
            host.set_read_timeout(Some(Duration::from_secs(120)))?;
            stream.set_read_timeout(Some(Duration::from_secs(120)))?;
            let mut host_read = host.try_clone()?;
            let mut viewer_write = stream.try_clone()?;
            let bridge = thread::spawn(move || {
                let _ = io::copy(&mut host_read, &mut viewer_write);
                let _ = viewer_write.shutdown(Shutdown::Both);
                let _ = host_read.shutdown(Shutdown::Both);
            });
            let _ = io::copy(&mut stream, &mut host);
            let _ = host.shutdown(Shutdown::Both);
            let _ = stream.shutdown(Shutdown::Both);
            let _ = bridge.join();
            Ok(())
        }
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid relay role",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairs_matching_channel_without_modifying_bytes() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let worker = thread::spawn(move || serve(listener, flag).unwrap());
        let channel = channel_id(&[42; 32]);
        let mut host = TcpStream::connect(address).unwrap();
        host.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        host.write_all(format!("ODESK1H{channel}").as_bytes())
            .unwrap();
        let mut waiting = [0; 6];
        host.read_exact(&mut waiting).unwrap();
        assert_eq!(&waiting, b"WAIT!\n");
        let mut viewer = TcpStream::connect(address).unwrap();
        viewer
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        viewer
            .write_all(format!("ODESK1V{channel}").as_bytes())
            .unwrap();
        let mut ready = [0; 6];
        host.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"READY\n");
        viewer.read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"READY\n");
        host.write_all(b"opaque ciphertext").unwrap();
        let mut data = [0; 17];
        viewer.read_exact(&mut data).unwrap();
        assert_eq!(&data, b"opaque ciphertext");
        viewer.write_all(b"reply").unwrap();
        let mut reply = [0; 5];
        host.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"reply");
        let _ = host.shutdown(Shutdown::Both);
        let _ = viewer.shutdown(Shutdown::Both);
        stop.store(true, Ordering::Relaxed);
        worker.join().unwrap();
    }
}
