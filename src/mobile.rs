//! Small C ABI used by the Flutter Android client. All connections use the same Rust protocol.
use crate::network::{self, Command, Connection, Event};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

static CONNECTIONS: OnceLock<Mutex<HashMap<u64, Connection>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
fn connections() -> &'static Mutex<HashMap<u64, Connection>> {
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}
unsafe fn string(pointer: *const c_char) -> Result<String, String> {
    if pointer.is_null() {
        return Err("Missing text argument".into());
    }
    unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map(str::to_owned)
        .map_err(|e| e.to_string())
}
/// Sends iroh's connection diagnostics to logcat (tag "opendesk"), once per process.
#[cfg(target_os = "android")]
fn init_logging() {
    use tracing_subscriber::{
        Layer, filter::Targets, layer::SubscriberExt, util::SubscriberInitExt,
    };
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let filter = Targets::new()
            .with_default(tracing::Level::INFO)
            .with_target("iroh", tracing::Level::DEBUG)
            .with_target("netwatch", tracing::Level::DEBUG)
            .with_target("opendesk", tracing::Level::DEBUG);
        let _ = tracing_subscriber::registry()
            .with(paranoid_android::layer("opendesk").with_filter(filter))
            .try_init();
    });
}
#[cfg(not(target_os = "android"))]
fn init_logging() {}
fn json_string(value: serde_json::Value) -> *mut c_char {
    CString::new(value.to_string()).map_or(std::ptr::null_mut(), CString::into_raw)
}

/// An empty endpoint without relay starts a code-only Internet session over iroh.
///
/// # Safety
/// endpoint and code must point to live, NUL-terminated UTF-8 C strings for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendesk_start(
    endpoint: *const c_char,
    code: *const c_char,
    relay: bool,
    host: bool,
) -> u64 {
    std::panic::catch_unwind(|| {
        let Ok(endpoint) = (unsafe { string(endpoint) }) else {
            return 0;
        };
        let Ok(code) = (unsafe { string(code) }) else {
            return 0;
        };
        let internet = !relay && endpoint.trim().is_empty();
        if internet {
            init_logging();
        }
        let result = match (host, relay) {
            _ if internet && host => crate::p2p::host(&code),
            _ if internet => crate::p2p::connect(&code),
            (true, true) => network::host_relay(&endpoint, &code),
            (false, true) => network::connect_relay(&endpoint, &code),
            (true, false) => network::host(&endpoint, &code),
            (false, false) => network::connect(&endpoint, &code),
        };
        match result {
            Ok(connection) => {
                let id = NEXT.fetch_add(1, Ordering::Relaxed);
                connections().lock().unwrap().insert(id, connection);
                id
            }
            Err(_) => 0,
        }
    })
    .unwrap_or(0)
}
#[unsafe(no_mangle)]
pub extern "C" fn opendesk_generate_code() -> *mut c_char {
    CString::new(network::generate_code()).map_or(std::ptr::null_mut(), CString::into_raw)
}

#[unsafe(no_mangle)]
pub extern "C" fn opendesk_poll(id: u64) -> *mut c_char {
    std::panic::catch_unwind(|| {
        let map = connections().lock().unwrap();
        let Some(c) = map.get(&id) else {
            return std::ptr::null_mut();
        };
        match c.events.try_recv() {
            Ok(Event::Frame { jpeg }) => {
                json_string(serde_json::json!({"Frame":{"jpeg64":STANDARD.encode(jpeg)}}))
            }
            Ok(event) => {
                // Later statuses overwrite earlier ones in the UI; keep each one in logcat.
                #[cfg(target_os = "android")]
                if let Event::Status(text) = &event {
                    tracing::info!(target: "opendesk", "status: {text}");
                } else if matches!(event, Event::Disconnected) {
                    tracing::info!(target: "opendesk", "disconnected");
                }
                serde_json::to_value(event).map_or(std::ptr::null_mut(), json_string)
            }
            Err(_) => std::ptr::null_mut(),
        }
    })
    .unwrap_or(std::ptr::null_mut())
}
/// # Safety
/// message must be a live NUL-terminated UTF-8 JSON string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendesk_command(id: u64, message: *const c_char) -> bool {
    std::panic::catch_unwind(|| {
        let Ok(text) = (unsafe { string(message) }) else {
            return false;
        };
        if text.len() > 65536 {
            return false;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            return false;
        };
        let command = match v["kind"].as_str() {
            Some("stop") => Command::Stop,
            Some("accept") => {
                let Ok(p) = serde_json::from_value(v["permissions"].clone()) else {
                    return false;
                };
                Command::AcceptPermissions(p)
            }
            Some("reject") => Command::Reject,
            Some("reject-transfer") => Command::RejectTransfer,
            Some("input") => {
                let Ok(input) = serde_json::from_value(v["event"].clone()) else {
                    return false;
                };
                Command::Input(input)
            }
            Some("chat") => Command::Chat(v["text"].as_str().unwrap_or_default().to_owned()),
            Some("clipboard") => {
                Command::Clipboard(v["text"].as_str().unwrap_or_default().to_owned())
            }
            Some("monitor") => {
                Command::SelectMonitor(v["id"].as_u64().unwrap_or_default() as usize)
            }
            Some("authenticate") => Command::Authenticate {
                password: v["password"].as_str().unwrap_or_default().to_owned(),
                totp: v["totp"].as_str().unwrap_or_default().to_owned(),
            },
            _ => return false,
        };
        connections()
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|c| c.commands.try_send(command).is_ok())
    })
    .unwrap_or(false)
}
/// # Safety
/// data must point to len readable bytes for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendesk_send_frame(id: u64, data: *const u8, len: usize) -> bool {
    if data.is_null() || len == 0 || len > 12 * 1024 * 1024 {
        return false;
    }
    std::panic::catch_unwind(|| {
        let frame = unsafe { std::slice::from_raw_parts(data, len) }.to_vec();
        connections()
            .lock()
            .unwrap()
            .get(&id)
            .is_some_and(|c| c.commands.try_send(Command::Frame(frame)).is_ok())
    })
    .unwrap_or(false)
}
#[unsafe(no_mangle)]
pub extern "C" fn opendesk_close(id: u64) {
    if let Ok(mut map) = connections().lock()
        && let Some(c) = map.remove(&id)
    {
        let _ = c.commands.try_send(Command::Stop);
    }
}
/// # Safety
/// pointer must be an unfreed string returned by this library, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendesk_string_free(pointer: *mut c_char) {
    if !pointer.is_null() {
        drop(unsafe { CString::from_raw(pointer) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_bridge_owns_and_releases_strings_and_sessions() {
        let code = opendesk_generate_code();
        assert!(!code.is_null());
        let endpoint = CString::new("127.0.0.1:0").unwrap();
        let handle = unsafe { opendesk_start(endpoint.as_ptr(), code, false, true) };
        unsafe { opendesk_string_free(code) };
        assert_ne!(handle, 0);
        let stop = CString::new(r#"{"kind":"stop"}"#).unwrap();
        assert!(unsafe { opendesk_command(handle, stop.as_ptr()) });
        opendesk_close(handle);
        assert!(opendesk_poll(handle).is_null());
        assert_eq!(
            unsafe { opendesk_start(std::ptr::null(), std::ptr::null(), false, false) },
            0
        );
    }
}
