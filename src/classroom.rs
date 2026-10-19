//! Classroom sessions: one teacher, many students on the same network.
//!
//! Students find the class through a UDP beacon (it carries only the class name and
//! port, never the code) and join over TCP with the class code. The code is the Noise
//! PSK, so only people who were told the code can join or read the traffic. Every link
//! carries small JSON control messages plus binary JPEG frames, which travel in both
//! directions: student screens to the teacher, the shared screen to the students.
//!
//! Each link uses a reader and a writer thread. Only the newest outgoing frame is kept,
//! so a slow student never builds a backlog or slows down the others.
use crate::network::{InputEvent, packet, read_packet, validate_input};
use rand::RngCore;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    collections::VecDeque,
    io::Read,
    net::{SocketAddr, TcpListener, TcpStream, UdpSocket},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

/// TCP port the teacher listens on.
pub const CLASS_PORT: u16 = 7880;
/// UDP port students listen on for class announcements.
pub const BEACON_PORT: u16 = 7881;
const MAX_FRAME: usize = 12 * 1024 * 1024;
const CHUNK: usize = 60_000;
const MAX_QUEUED: usize = 256;
const PING_EVERY: Duration = Duration::from_secs(5);
const SILENCE_LIMIT: Duration = Duration::from_secs(20);
const KIND_MESSAGE: u8 = 0;
const KIND_FRAME_START: u8 = 1;
const KIND_CHUNK: u8 = 2;
const KIND_PING: u8 = 3;
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Messages the teacher sends to a student.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToStudent {
    Welcome {
        class: String,
    },
    /// Screen capture the teacher wants from this student; `fps == 0` pauses it.
    Capture {
        fps: u8,
        quality: u8,
        max_width: u32,
    },
    /// The teacher starts showing a screen (frames follow) with this title.
    ShowStart {
        title: String,
    },
    ShowEnd,
    Lock {
        locked: bool,
        message: String,
    },
    Message(String),
    OpenUrl(String),
    /// Remote control of the student's computer: `Control(false)` revokes it.
    Control(bool),
    Input(InputEvent),
    Bye,
}

/// Messages a student sends to the teacher.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ToTeacher {
    Hello { name: String, computer: String },
    Hand(bool),
    Message(String),
    Bye,
}

pub enum LinkEvent<T> {
    Message(T),
    Frame(Vec<u8>),
    Closed(String),
}

#[derive(Default)]
struct Outbox<T> {
    messages: VecDeque<T>,
    frame: Option<Arc<Vec<u8>>>,
    closed: bool,
}

/// One encrypted teacher–student connection. Dropping it closes the connection.
pub struct Link<Out, In> {
    events: Mutex<Receiver<LinkEvent<In>>>,
    pub peer: SocketAddr,
    outbox: Arc<(Mutex<Outbox<Out>>, Condvar)>,
    stream: TcpStream,
}
pub type StudentLink = Link<ToStudent, ToTeacher>;
pub type TeacherLink = Link<ToTeacher, ToStudent>;

impl<Out, In> Link<Out, In>
where
    Out: Serialize + Send + 'static,
    In: DeserializeOwned + Send + 'static,
{
    fn start(stream: TcpStream, noise: snow::StatelessTransportState) -> Result<Self, String> {
        let peer = stream.peer_addr().map_err(|e| e.to_string())?;
        stream
            .set_read_timeout(Some(SILENCE_LIMIT))
            .map_err(|e| e.to_string())?;
        stream
            .set_write_timeout(Some(SILENCE_LIMIT))
            .map_err(|e| e.to_string())?;
        let noise = Arc::new(noise);
        let outbox = Arc::new((
            Mutex::new(Outbox::<Out> {
                messages: VecDeque::new(),
                frame: None,
                closed: false,
            }),
            Condvar::new(),
        ));
        let (tx, events) = mpsc::sync_channel(64);
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        {
            let noise = noise.clone();
            let outbox = outbox.clone();
            thread::spawn(move || {
                let reason = read_loop(reader, &noise, &tx).err().unwrap_or_default();
                close(&outbox);
                let _ = tx.send(LinkEvent::Closed(reason));
            });
        }
        {
            let outbox = outbox.clone();
            thread::spawn(move || {
                let mut stream = writer;
                let _ = write_loop(&mut stream, &noise, &outbox);
                let _ = stream.shutdown(std::net::Shutdown::Both);
            });
        }
        Ok(Self {
            events: Mutex::new(events),
            peer,
            outbox,
            stream,
        })
    }
    /// Everything received since the last call, without waiting.
    pub fn poll_events(&self) -> Vec<LinkEvent<In>> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.try_iter().collect()
    }
    /// Waits up to `timeout` for the next event.
    pub fn next_event(&self, timeout: Duration) -> Option<LinkEvent<In>> {
        let events = self.events.lock().unwrap_or_else(|e| e.into_inner());
        events.recv_timeout(timeout).ok()
    }
    /// Queues a control message; false when the link is closed or badly backed up.
    pub fn send(&self, message: Out) -> bool {
        let (lock, wake) = &*self.outbox;
        let mut outbox = lock.lock().unwrap_or_else(|e| e.into_inner());
        if outbox.closed || outbox.messages.len() >= MAX_QUEUED {
            return false;
        }
        outbox.messages.push_back(message);
        wake.notify_one();
        true
    }
    /// Replaces any frame still waiting to be sent, so only the newest one goes out.
    pub fn send_frame(&self, jpeg: Arc<Vec<u8>>) {
        let (lock, wake) = &*self.outbox;
        let mut outbox = lock.lock().unwrap_or_else(|e| e.into_inner());
        if !outbox.closed {
            outbox.frame = Some(jpeg);
            wake.notify_one();
        }
    }
    /// True while an earlier frame has not been handed to the network yet.
    pub fn frame_pending(&self) -> bool {
        let outbox = self.outbox.0.lock().unwrap_or_else(|e| e.into_inner());
        outbox.frame.is_some()
    }
}
impl<Out, In> Drop for Link<Out, In> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.outbox;
        let mut outbox = lock.lock().unwrap_or_else(|e| e.into_inner());
        // Let the writer flush a final Bye before the socket goes away.
        outbox.closed = true;
        wake.notify_all();
        drop(outbox);
        let stream = self.stream.try_clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(300));
            if let Ok(stream) = stream {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        });
    }
}

fn close<T>(outbox: &Arc<(Mutex<Outbox<T>>, Condvar)>) {
    let (lock, wake) = &**outbox;
    lock.lock().unwrap_or_else(|e| e.into_inner()).closed = true;
    wake.notify_all();
}

fn seal(
    stream: &mut TcpStream,
    noise: &snow::StatelessTransportState,
    nonce: &mut u64,
    kind: u8,
    payload: &[u8],
) -> Result<(), String> {
    let mut plain = Vec::with_capacity(payload.len() + 1);
    plain.push(kind);
    plain.extend_from_slice(payload);
    let mut cipher = vec![0; plain.len() + 16];
    let n = noise
        .write_message(*nonce, &plain, &mut cipher)
        .map_err(|e| e.to_string())?;
    *nonce += 1;
    packet(stream, &cipher[..n])
}

fn write_loop<Out: Serialize>(
    stream: &mut TcpStream,
    noise: &snow::StatelessTransportState,
    outbox: &Arc<(Mutex<Outbox<Out>>, Condvar)>,
) -> Result<(), String> {
    let mut nonce = 0u64;
    let mut last_sent = Instant::now();
    loop {
        let (messages, frame, closed) = {
            let (lock, wake) = &**outbox;
            let mut guard = lock.lock().unwrap_or_else(|e| e.into_inner());
            while guard.messages.is_empty()
                && guard.frame.is_none()
                && !guard.closed
                && last_sent.elapsed() < PING_EVERY
            {
                let wait = PING_EVERY.saturating_sub(last_sent.elapsed());
                guard = wake
                    .wait_timeout(guard, wait)
                    .unwrap_or_else(|e| e.into_inner())
                    .0;
            }
            (
                std::mem::take(&mut guard.messages),
                guard.frame.take(),
                guard.closed,
            )
        };
        for message in &messages {
            let json = serde_json::to_vec(message).map_err(|e| e.to_string())?;
            if json.len() > CHUNK {
                continue;
            }
            seal(stream, noise, &mut nonce, KIND_MESSAGE, &json)?;
        }
        if closed {
            return Ok(());
        }
        if let Some(jpeg) = frame {
            seal(
                stream,
                noise,
                &mut nonce,
                KIND_FRAME_START,
                &(jpeg.len() as u32).to_be_bytes(),
            )?;
            for chunk in jpeg.chunks(CHUNK) {
                seal(stream, noise, &mut nonce, KIND_CHUNK, chunk)?;
            }
        }
        if messages.is_empty() && last_sent.elapsed() >= PING_EVERY {
            seal(stream, noise, &mut nonce, KIND_PING, &[])?;
        }
        last_sent = Instant::now();
    }
}

fn read_loop<In: DeserializeOwned>(
    mut stream: TcpStream,
    noise: &snow::StatelessTransportState,
    events: &SyncSender<LinkEvent<In>>,
) -> Result<(), String> {
    let mut nonce = 0u64;
    let mut plain = vec![0u8; 65535];
    let mut frame: Option<(usize, Vec<u8>)> = None;
    loop {
        let mut header = [0u8; 4];
        stream.read_exact(&mut header).map_err(describe)?;
        let n = u32::from_be_bytes(header) as usize;
        if !(17..=65535).contains(&n) {
            return Err("Invalid packet length".into());
        }
        let mut cipher = vec![0u8; n];
        stream.read_exact(&mut cipher).map_err(describe)?;
        let count = noise
            .read_message(nonce, &cipher, &mut plain)
            .map_err(|_| "Invalid encrypted record")?;
        nonce += 1;
        let (kind, payload) = plain[..count].split_first().ok_or("Empty record")?;
        match *kind {
            KIND_PING => {}
            KIND_MESSAGE => {
                let message: In =
                    serde_json::from_slice(payload).map_err(|_| "Invalid class message")?;
                if events.send(LinkEvent::Message(message)).is_err() {
                    return Ok(());
                }
            }
            KIND_FRAME_START => {
                let size =
                    u32::from_be_bytes(payload.try_into().map_err(|_| "Invalid frame header")?)
                        as usize;
                if size == 0 || size > MAX_FRAME {
                    return Err("Frame exceeds limit".into());
                }
                frame = Some((size, Vec::with_capacity(size)));
            }
            KIND_CHUNK => {
                let Some((size, bytes)) = frame.as_mut() else {
                    return Err("Frame chunk without header".into());
                };
                if bytes.len() + payload.len() > *size {
                    return Err("Invalid frame chunk".into());
                }
                bytes.extend_from_slice(payload);
                if bytes.len() == *size {
                    let (_, bytes) = frame.take().unwrap_or_default();
                    // A consumer that is behind skips frames instead of queueing them.
                    if let Err(mpsc::TrySendError::Disconnected(_)) =
                        events.try_send(LinkEvent::Frame(bytes))
                    {
                        return Ok(());
                    }
                }
            }
            _ => return Err("Unknown record".into()),
        }
    }
}

fn describe(error: std::io::Error) -> String {
    match error.kind() {
        std::io::ErrorKind::UnexpectedEof => "Connection closed".into(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            "Connection timed out".into()
        }
        _ => error.to_string(),
    }
}

/// Short class codes, `ABCDE-FGHJK`: 50 random bits, easy to read off a projector.
pub fn generate_class_code() -> String {
    let mut random = [0u8; 10];
    rand::thread_rng().fill_bytes(&mut random);
    random
        .chunks(5)
        .map(|group| {
            group
                .iter()
                .map(|b| ALPHABET[(b & 31) as usize] as char)
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("-")
}

pub fn normalize_code(code: &str) -> Option<String> {
    let normalized: String = code
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();
    (normalized.len() == 10 && normalized.bytes().all(|b| ALPHABET.contains(&b)))
        .then_some(normalized)
}

fn class_key(code: &str) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let code = normalize_code(code).ok_or("Class code has 10 letters/digits, e.g. ABCDE-FGHJK")?;
    let mut hash = Sha256::new();
    hash.update(b"Opendesk class v1");
    hash.update(code.as_bytes());
    Ok(hash.finalize().into())
}

fn handshake(
    stream: &mut TcpStream,
    teacher: bool,
    key: &[u8; 32],
) -> Result<snow::StatelessTransportState, String> {
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let builder = snow::Builder::new(
        "Noise_NNpsk0_25519_ChaChaPoly_BLAKE2s"
            .parse()
            .map_err(|e: snow::Error| e.to_string())?,
    )
    .psk(0, key)
    .prologue(b"Opendesk class v1");
    let mut hs = if teacher {
        builder.build_responder()
    } else {
        builder.build_initiator()
    }
    .map_err(|e| e.to_string())?;
    let mut buf = [0u8; 65535];
    if !teacher {
        let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
        packet(stream, &buf[..n])?;
    }
    let message = read_packet(stream)?;
    hs.read_message(&message, &mut buf)
        .map_err(|_| "Wrong class code".to_string())?;
    if teacher {
        let n = hs.write_message(&[], &mut buf).map_err(|e| e.to_string())?;
        packet(stream, &buf[..n])?;
    }
    hs.into_stateless_transport_mode()
        .map_err(|e| e.to_string())
}

#[derive(Serialize, Deserialize)]
struct Beacon {
    opendesk_class: u8,
    name: String,
    port: u16,
}

/// A running class: accepts students and announces itself on the local network.
pub struct ClassServer {
    pub joined: Receiver<StudentLink>,
    pub port: u16,
    stop: Arc<AtomicBool>,
}
impl Drop for ClassServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn serve(port: u16, code: &str, class_name: &str) -> Result<ClassServer, String> {
    let key = class_key(code)?;
    let listener = TcpListener::bind(("0.0.0.0", port)).map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse => format!("Port {port} is already in use"),
        _ => e.to_string(),
    })?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, joined) = mpsc::sync_channel(64);
    {
        let stop = stop.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let tx = tx.clone();
                        thread::spawn(move || {
                            if let Ok(noise) = handshake(&mut stream, true, &key)
                                && let Ok(link) = Link::start(stream, noise)
                            {
                                let _ = tx.send(link);
                            }
                        });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(50))
                    }
                    Err(_) => thread::sleep(Duration::from_millis(200)),
                }
            }
        });
    }
    {
        let stop = stop.clone();
        let beacon = serde_json::to_vec(&Beacon {
            opendesk_class: 1,
            name: class_name.chars().take(64).collect(),
            port,
        })
        .map_err(|e| e.to_string())?;
        thread::spawn(move || {
            let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
                return;
            };
            let _ = socket.set_broadcast(true);
            while !stop.load(Ordering::Relaxed) {
                let _ = socket.send_to(&beacon, ("255.255.255.255", BEACON_PORT));
                if let Some(std::net::IpAddr::V4(ip)) = crate::network::local_ip() {
                    // Directed /24 broadcast reaches classrooms where the limited
                    // broadcast leaves through another adapter.
                    let [a, b, c, _] = ip.octets();
                    let directed = std::net::Ipv4Addr::new(a, b, c, 255);
                    let _ = socket.send_to(&beacon, (directed, BEACON_PORT));
                }
                for _ in 0..20 {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(100));
                }
            }
        });
    }
    Ok(ClassServer { joined, port, stop })
}

/// Joins a class at `address` (IP or IP:port) with the class code.
pub fn join(address: &str, code: &str) -> Result<TeacherLink, String> {
    use std::net::ToSocketAddrs;
    let key = class_key(code)?;
    let address = address.trim();
    let target = if address.contains(':') && !address.contains("::") {
        address.to_owned()
    } else {
        format!("{address}:{CLASS_PORT}")
    };
    let endpoint = target
        .to_socket_addrs()
        .map_err(|e| e.to_string())?
        .next()
        .ok_or("Address has no endpoint")?;
    let mut stream =
        TcpStream::connect_timeout(&endpoint, Duration::from_secs(4)).map_err(|e| e.to_string())?;
    let noise = handshake(&mut stream, false, &key)?;
    Link::start(stream, noise)
}

/// Checks a teacher's input event before it is injected on a student computer.
pub fn check_input(input: &InputEvent) -> Result<(), String> {
    validate_input(input)
}

/// A class announced on the local network.
#[derive(Debug, Clone, PartialEq)]
pub struct Announced {
    pub name: String,
    pub address: SocketAddr,
}

/// Listens for class announcements until dropped.
pub struct Discovery {
    pub found: Receiver<Announced>,
    stop: Arc<AtomicBool>,
}
impl Drop for Discovery {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn discover() -> Result<Discovery, String> {
    let socket = UdpSocket::bind(("0.0.0.0", BEACON_PORT)).map_err(|e| e.to_string())?;
    socket
        .set_read_timeout(Some(Duration::from_millis(300)))
        .map_err(|e| e.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let (tx, found) = mpsc::sync_channel(32);
    {
        let stop = stop.clone();
        thread::spawn(move || {
            let mut buffer = [0u8; 512];
            while !stop.load(Ordering::Relaxed) {
                let Ok((n, from)) = socket.recv_from(&mut buffer) else {
                    continue;
                };
                let Ok(beacon) = serde_json::from_slice::<Beacon>(&buffer[..n]) else {
                    continue;
                };
                if beacon.opendesk_class != 1 || beacon.port == 0 {
                    continue;
                }
                let _ = tx.try_send(Announced {
                    name: beacon.name.chars().take(64).collect(),
                    address: SocketAddr::new(from.ip(), beacon.port),
                });
            }
        });
    }
    Ok(Discovery { found, stop })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn next<O: Serialize + Send + 'static, T: DeserializeOwned + Send + 'static>(
        link: &Link<O, T>,
    ) -> LinkEvent<T> {
        link.next_event(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn class_codes_are_short_and_normalized() {
        let code = generate_class_code();
        assert_eq!(code.len(), 11);
        assert_eq!(normalize_code(&code.to_lowercase()), normalize_code(&code));
        assert!(normalize_code("ABCDE-FGHJ1").is_none());
        assert!(normalize_code("ABC").is_none());
    }

    #[test]
    fn teacher_and_student_exchange_messages_and_frames() {
        let code = generate_class_code();
        let server = serve(0, &code, "Test class").unwrap();
        let student = join(&format!("127.0.0.1:{}", server.port), &code).unwrap();
        let teacher = server.joined.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(student.send(ToTeacher::Hello {
            name: "Ada".into(),
            computer: "PC-1".into()
        }));
        assert!(matches!(
            next(&teacher),
            LinkEvent::Message(ToTeacher::Hello { name, .. }) if name == "Ada"
        ));
        let jpeg: Vec<u8> = (0..150_000u32).map(|i| i as u8).collect();
        teacher.send(ToStudent::ShowStart {
            title: "Teacher".into(),
        });
        teacher.send_frame(Arc::new(jpeg.clone()));
        assert!(matches!(
            next(&student),
            LinkEvent::Message(ToStudent::ShowStart { .. })
        ));
        assert!(matches!(next(&student), LinkEvent::Frame(f) if f == jpeg));
        student.send_frame(Arc::new(vec![7; 10]));
        assert!(matches!(next(&teacher), LinkEvent::Frame(f) if f == vec![7; 10]));
        drop(student);
        loop {
            if let LinkEvent::Closed(_) = next(&teacher) {
                break;
            }
        }
    }

    #[test]
    fn wrong_code_cannot_join() {
        let server = serve(0, &generate_class_code(), "Test class").unwrap();
        let other = generate_class_code();
        assert!(join(&format!("127.0.0.1:{}", server.port), &other).is_err());
        assert!(
            server
                .joined
                .recv_timeout(Duration::from_millis(500))
                .is_err()
        );
    }
}
