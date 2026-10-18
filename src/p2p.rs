//! Code-addressed Internet sessions over iroh: QUIC with UDP hole punching, and n0's free
//! public relays when no direct path exists. Nobody types an IP address or runs a server.
//!
//! The iroh endpoint key is derived from the pairing code, so only code holders can find
//! the host. The QUIC stream is bridged to a loopback socket, so the Noise PSK session,
//! explicit consent and permissions from `network` run unchanged on top of iroh's TLS.
use crate::network::{self, Connection, Event};
use iroh::{Endpoint, EndpointId, SecretKey, endpoint::presets};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::{SocketAddr, TcpListener},
    sync::{Arc, Mutex, mpsc::SyncSender},
    thread,
    time::Duration,
};
use tokio::sync::oneshot;

const ALPN: &[u8] = b"opendesk/p2p/2";
/// Unauthenticated tunnels a host bridges at once; each still has to pass the Noise handshake.
const MAX_PENDING: usize = 4;
const DIAL_TIMEOUT: Duration = Duration::from_secs(30);
const BIND_TIMEOUT: Duration = Duration::from_secs(15);

/// Dropping the guard shuts the iroh endpoint and its bridges down.
pub struct Guard {
    _stop: oneshot::Sender<()>,
}

type Labels = Arc<Mutex<HashMap<SocketAddr, String>>>;

fn endpoint_secret(key: &[u8; 32]) -> SecretKey {
    let mut hash = Sha256::new();
    hash.update(b"Opendesk iroh endpoint v1");
    hash.update(key);
    SecretKey::from_bytes(&hash.finalize().into())
}

/// The public iroh endpoint id a host with this pairing code listens on.
pub fn endpoint_id(code: &str) -> Result<EndpointId, String> {
    Ok(endpoint_secret(&network::secret(code)?).public())
}

fn status(events: &SyncSender<Event>, text: impl Into<String>) {
    let _ = events.try_send(Event::Status(text.into()));
}

fn spawn_runtime(
    task: impl FnOnce(oneshot::Receiver<()>) -> std::pin::Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + 'static,
) -> Result<Guard, String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let (stop, stopped) = oneshot::channel();
    thread::spawn(move || runtime.block_on(task(stopped)));
    Ok(Guard { _stop: stop })
}

/// Shares this computer under its pairing code. Viewers need only the same code.
pub fn host(code: &str) -> Result<Connection, String> {
    let key = network::secret(code)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    let labels = Labels::default();
    let lookup = labels.clone();
    let (mut connection, events) = network::local_listener(listener, key, true, move |peer| {
        lookup
            .lock()
            .unwrap()
            .get(&peer)
            .cloned()
            .unwrap_or_else(|| peer.to_string())
    })?;
    let secret = endpoint_secret(&key);
    connection.tunnel = Some(spawn_runtime(move |stop| {
        Box::pin(serve(secret, local, labels, events, stop))
    })?);
    Ok(connection)
}

async fn serve(
    secret: SecretKey,
    local: SocketAddr,
    labels: Labels,
    events: SyncSender<Event>,
    mut stop: oneshot::Receiver<()>,
) {
    status(&events, "Joining the iroh network…");
    let bind = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind();
    let endpoint = match tokio::time::timeout(BIND_TIMEOUT, bind).await {
        Ok(Ok(endpoint)) => endpoint,
        Ok(Err(e)) => {
            status(&events, format!("iroh unavailable: {e}"));
            return;
        }
        Err(_) => {
            status(&events, "iroh could not open a network endpoint in time");
            return;
        }
    };
    tokio::select! {
        _ = &mut stop => {
            endpoint.close().await;
            return;
        }
        online = tokio::time::timeout(Duration::from_secs(15), endpoint.online()) => {
            status(&events, if online.is_ok() {
                "Waiting for connection: reachable by code over the Internet"
            } else {
                "iroh network not reachable yet; still trying"
            });
        }
    }
    let permits = Arc::new(tokio::sync::Semaphore::new(MAX_PENDING));
    loop {
        tokio::select! {
            _ = &mut stop => break,
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    incoming.refuse();
                    continue;
                };
                let labels = labels.clone();
                let events = events.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(e) = bridge_incoming(incoming, local, labels, events.clone()).await {
                        status(&events, format!("Ignored Internet connection: {e}"));
                    }
                });
            }
        }
    }
    endpoint.close().await;
}

async fn bridge_incoming(
    incoming: iroh::endpoint::Incoming,
    local: SocketAddr,
    labels: Labels,
    events: SyncSender<Event>,
) -> Result<(), String> {
    let connection = tokio::time::timeout(Duration::from_secs(10), incoming)
        .await
        .map_err(|_| "handshake timed out")?
        .map_err(|e| e.to_string())?;
    let (send, recv) = tokio::time::timeout(Duration::from_secs(10), connection.accept_bi())
        .await
        .map_err(|_| "no session stream")?
        .map_err(|e| e.to_string())?;
    let socket = tokio::net::TcpSocket::new_v4().map_err(|e| e.to_string())?;
    socket
        .bind("127.0.0.1:0".parse().unwrap())
        .map_err(|e| e.to_string())?;
    let address = socket.local_addr().map_err(|e| e.to_string())?;
    labels.lock().unwrap().insert(
        address,
        format!("Internet peer {}", connection.remote_id().fmt_short()),
    );
    let result = async {
        let mut tcp = socket.connect(local).await.map_err(|e| e.to_string())?;
        let mut quic = tokio::io::join(recv, send);
        tokio::select! {
            _ = tokio::io::copy_bidirectional(&mut tcp, &mut quic) => {}
            _ = report_paths(&connection, &events) => {}
        }
        Ok(())
    }
    .await;
    labels.lock().unwrap().remove(&address);
    connection.close(0u32.into(), b"session ended");
    result
}

/// Tells the user whether the session runs directly or through a public relay.
async fn report_paths(connection: &iroh::endpoint::Connection, events: &SyncSender<Event>) {
    let mut last = None;
    loop {
        let selected = connection
            .paths()
            .iter()
            .find(|path| path.is_selected())
            .map(|path| (path.is_relay(), path.rtt().as_millis()));
        if let Some((relay, rtt)) = selected
            && last != Some(relay)
        {
            last = Some(relay);
            status(
                events,
                if relay {
                    format!("Internet path: public relay, {rtt} ms")
                } else {
                    format!("Internet path: direct, {rtt} ms")
                },
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Finds the host that shares `code` and opens a session to it, with no address needed.
pub fn connect(code: &str) -> Result<Connection, String> {
    let key = network::secret(code)?;
    let target = endpoint_secret(&key).public();
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let local = listener.local_addr().map_err(|e| e.to_string())?;
    let (mut connection, events) = network::local_listener(listener, key, false, move |_| {
        format!("Internet host {}", target.fmt_short())
    })?;
    connection.tunnel = Some(spawn_runtime(move |stop| {
        Box::pin(dial(target, local, events, stop))
    })?);
    Ok(connection)
}

async fn dial(
    target: EndpointId,
    local: SocketAddr,
    events: SyncSender<Event>,
    mut stop: oneshot::Receiver<()>,
) {
    status(&events, "Opening Internet connection…");
    let endpoint = match tokio::time::timeout(BIND_TIMEOUT, Endpoint::bind(presets::N0)).await {
        Ok(Ok(endpoint)) => endpoint,
        Ok(Err(e)) => {
            status(&events, format!("iroh unavailable: {e}"));
            let _ = events.try_send(Event::Disconnected);
            return;
        }
        Err(_) => {
            status(&events, "iroh could not open a network endpoint in time");
            let _ = events.try_send(Event::Disconnected);
            return;
        }
    };
    status(&events, "Looking for the host on the Internet…");
    let result = async {
        let connection = tokio::time::timeout(DIAL_TIMEOUT, endpoint.connect(target, ALPN))
            .await
            .map_err(|_| "Host not found. Is it sharing with the same code?".to_string())?
            .map_err(|e| format!("Host not reachable: {e}"))?;
        let (send, recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
        let mut tcp = tokio::net::TcpStream::connect(local)
            .await
            .map_err(|e| e.to_string())?;
        tcp.set_nodelay(true).map_err(|e| e.to_string())?;
        let mut quic = tokio::io::join(recv, send);
        tokio::select! {
            _ = tokio::io::copy_bidirectional(&mut tcp, &mut quic) => {}
            _ = report_paths(&connection, &events) => {}
        }
        connection.close(0u32.into(), b"session ended");
        Ok::<(), String>(())
    };
    let outcome = tokio::select! {
        outcome = result => outcome,
        _ = &mut stop => Ok(()),
    };
    if let Err(e) = outcome {
        status(&events, e);
        // The loopback session never started; let the UI end the attempt.
        let _ = events.try_send(Event::Disconnected);
    }
    endpoint.close().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_id_depends_only_on_pairing_code() {
        let code = network::generate_code();
        assert_eq!(
            endpoint_id(&code).unwrap(),
            endpoint_id(&code.to_lowercase()).unwrap()
        );
        assert_ne!(
            endpoint_id(&code).unwrap(),
            endpoint_id(&network::generate_code()).unwrap()
        );
        assert!(endpoint_id("123456").is_err());
    }

    fn next(c: &Connection, wanted: impl Fn(&Event) -> bool) -> Event {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            let event = c.events.recv_timeout(left).expect("event before timeout");
            if let Event::Status(s) = &event {
                eprintln!("status: {s}");
            }
            if wanted(&event) {
                return event;
            }
            assert!(!matches!(event, Event::Disconnected), "disconnected early");
        }
    }

    /// Uses n0's public address lookup and relays: `cargo test -- --ignored p2p`.
    #[test]
    #[ignore = "needs Internet access"]
    fn p2p_session_by_code_only() {
        use network::Command;
        let code = network::generate_code();
        let h = host(&code).unwrap();
        next(
            &h,
            |e| matches!(e, Event::Status(s) if s.starts_with("Waiting for connection: reachable")),
        );
        let c = connect(&code).unwrap();
        next(&h, |e| matches!(e, Event::Incoming { .. }));
        h.commands.send(Command::Accept { control: false }).unwrap();
        next(&c, |e| matches!(e, Event::Connected { .. }));
        c.commands
            .send(Command::Clipboard("over iroh".into()))
            .unwrap();
        next(&h, |e| matches!(e, Event::Clipboard(s) if s == "over iroh"));
        h.commands.send(Command::Frame(vec![7; 300_000])).unwrap();
        let started = std::time::Instant::now();
        next(
            &c,
            |e| matches!(e, Event::Frame { jpeg } if jpeg.len() == 300_000),
        );
        eprintln!("300 kB frame in {:?}", started.elapsed());
        c.commands.send(Command::Stop).unwrap();
        next(&h, |e| matches!(e, Event::Disconnected));
    }
}
