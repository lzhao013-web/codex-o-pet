use std::{collections::VecDeque, env, io, io::Write, path::PathBuf};

use interprocess::local_socket::{GenericFilePath, Stream, ToFsName as _, traits::Stream as _};
use serde_json::{Value, json};

use crate::mcp::PetTransport;

const ENDPOINT_ENV: &str = "O_PET_ENDPOINT";
const CLIENT_ID: &str = "codex-o-pet";
const MAX_PENDING_EVENTS: usize = 256;

pub struct PetClient {
    endpoint: PathBuf,
    session_id: Option<String>,
    stream: Option<Stream>,
    pending_events: VecDeque<Value>,
}

impl PetClient {
    pub fn from_environment() -> io::Result<Self> {
        Ok(Self::new(resolve_endpoint()?))
    }

    pub fn new(endpoint: PathBuf) -> Self {
        Self {
            endpoint,
            session_id: None,
            stream: None,
            pending_events: VecDeque::new(),
        }
    }

    fn bind_session(&mut self, session_id: &str) {
        if self.session_id.as_deref() != Some(session_id) {
            self.close();
            self.pending_events.clear();
            self.session_id = Some(session_id.to_string());
        }
    }

    fn enqueue(&mut self, events: &[Value]) {
        self.pending_events.extend(events.iter().cloned());
        let overflow = self.pending_events.len().saturating_sub(MAX_PENDING_EVENTS);
        self.pending_events.drain(..overflow);
    }

    fn connect(&mut self) -> io::Result<()> {
        let session_id = self
            .session_id
            .as_deref()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing session id"))?;
        let name = self.endpoint.as_os_str().to_fs_name::<GenericFilePath>()?;
        let mut stream = Stream::connect(name)?;
        write_line(
            &mut stream,
            &json!({
                "type": "hello",
                "clientId": CLIENT_ID,
                "sessionId": session_id,
            }),
        )?;
        self.stream = Some(stream);
        Ok(())
    }

    fn flush_pending(&mut self) -> io::Result<()> {
        while let Some(event) = self.pending_events.front() {
            let stream = self.stream.as_mut().ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotConnected, "o-pet is not connected")
            })?;
            write_line(
                stream,
                &json!({
                    "type": "event",
                    "event": event,
                }),
            )?;
            self.pending_events.pop_front();
        }
        Ok(())
    }

    fn connect_and_flush(&mut self) -> io::Result<()> {
        if self.stream.is_none() {
            self.connect()?;
        }
        self.flush_pending()
    }

    fn close(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            let _ = write_line(&mut stream, &json!({ "type": "goodbye" }));
        }
    }

    #[cfg(test)]
    fn shutdown(&mut self) {
        self.close();
    }
}

impl PetTransport for PetClient {
    fn deliver(&mut self, session_id: &str, events: &[Value]) -> io::Result<()> {
        self.bind_session(session_id);
        self.enqueue(events);
        if let Err(first_error) = self.connect_and_flush() {
            self.stream = None;
            self.connect_and_flush().map_err(|retry_error| {
                io::Error::new(
                    retry_error.kind(),
                    format!(
                        "o-pet delivery failed ({first_error}); reconnect failed ({retry_error})"
                    ),
                )
            })?;
        }
        Ok(())
    }
}

impl Drop for PetClient {
    fn drop(&mut self) {
        self.close();
    }
}

fn write_line(stream: &mut Stream, value: &Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(io::Error::other)?;
    line.push(b'\n');
    stream.write_all(&line)
}

pub fn resolve_endpoint() -> io::Result<PathBuf> {
    if let Some(endpoint) = env::var_os(ENDPOINT_ENV).filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(endpoint));
    }
    default_endpoint()
}

#[cfg(target_os = "linux")]
fn default_endpoint() -> io::Result<PathBuf> {
    let runtime_directory = env::var_os("XDG_RUNTIME_DIR")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "o-pet requires XDG_RUNTIME_DIR"))?;
    Ok(PathBuf::from(runtime_directory).join("o-pet.sock"))
}

#[cfg(target_os = "macos")]
fn default_endpoint() -> io::Result<PathBuf> {
    let uid = unsafe { libc::getuid() };
    Ok(env::temp_dir()
        .join(format!("o-pet-{uid}"))
        .join("o-pet.sock"))
}

#[cfg(windows)]
fn default_endpoint() -> io::Result<PathBuf> {
    use std::fmt::Write as _;

    use sha2::{Digest, Sha256};

    let username = required_environment("USERNAME")?;
    let home = required_environment("USERPROFILE")?;
    let mut digest = Sha256::new();
    digest.update(username.to_string_lossy().as_bytes());
    digest.update([0]);
    digest.update(home.to_string_lossy().as_bytes());
    let digest = digest.finalize();
    let mut identity = String::with_capacity(16);
    for byte in &digest[..8] {
        write!(&mut identity, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Ok(PathBuf::from(format!(r"\\.\pipe\o-pet-{identity}")))
}

#[cfg(windows)]
fn required_environment(name: &str) -> io::Result<std::ffi::OsString> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("missing {name}")))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn default_endpoint() -> io::Result<PathBuf> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "o-pet does not support this platform",
    ))
}

#[cfg(test)]
mod tests {
    use std::{io::Read, thread};

    #[cfg(windows)]
    use std::sync::atomic::{AtomicU64, Ordering};

    use interprocess::local_socket::{ListenerOptions, traits::Listener as _};
    use serde_json::json;

    use super::*;

    #[cfg(windows)]
    static NEXT_ENDPOINT: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn writes_hello_events_and_goodbye_over_the_real_local_transport() {
        let (_directory, endpoint) = test_endpoint();
        let name = endpoint
            .as_os_str()
            .to_fs_name::<GenericFilePath>()
            .expect("valid test endpoint");
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .expect("create local listener");
        let reader = thread::spawn(move || {
            let mut stream = listener.accept().expect("accept bridge connection");
            let mut text = String::new();
            stream
                .read_to_string(&mut text)
                .expect("read bridge messages");
            text
        });

        let mut client = PetClient::new(endpoint);
        client
            .deliver("session-1", &[json!({ "type": "thinking_started" })])
            .expect("deliver event");
        client.shutdown();

        let messages = reader
            .join()
            .expect("reader thread")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid JSON line"))
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![
                json!({
                    "type": "hello",
                    "clientId": "codex-o-pet",
                    "sessionId": "session-1",
                }),
                json!({
                    "type": "event",
                    "event": { "type": "thinking_started" },
                }),
                json!({ "type": "goodbye" }),
            ]
        );
    }

    #[test]
    fn replays_events_queued_before_o_pet_starts() {
        let (_directory, endpoint) = test_endpoint();
        let mut client = PetClient::new(endpoint.clone());
        assert!(
            client
                .deliver("session-1", &[json!({ "type": "agent_started" })])
                .is_err()
        );

        let name = endpoint
            .as_os_str()
            .to_fs_name::<GenericFilePath>()
            .expect("valid test endpoint");
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .expect("create local listener");
        let reader = thread::spawn(move || {
            let mut stream = listener.accept().expect("accept bridge connection");
            let mut text = String::new();
            stream
                .read_to_string(&mut text)
                .expect("read bridge messages");
            text
        });

        client
            .deliver("session-1", &[json!({ "type": "turn_started" })])
            .expect("reconnect and replay queued events");
        client.shutdown();

        let messages = reader
            .join()
            .expect("reader thread")
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).expect("valid JSON line"))
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![
                json!({
                    "type": "hello",
                    "clientId": "codex-o-pet",
                    "sessionId": "session-1",
                }),
                json!({
                    "type": "event",
                    "event": { "type": "agent_started" },
                }),
                json!({
                    "type": "event",
                    "event": { "type": "turn_started" },
                }),
                json!({ "type": "goodbye" }),
            ]
        );
    }

    #[test]
    fn bounds_pending_events_by_dropping_the_oldest() {
        let (_directory, endpoint) = test_endpoint();
        let mut client = PetClient::new(endpoint);
        client.bind_session("session-1");

        let events = (0..MAX_PENDING_EVENTS + 2)
            .map(|sequence| json!({ "sequence": sequence }))
            .collect::<Vec<_>>();
        client.enqueue(&events);

        assert_eq!(client.pending_events.len(), MAX_PENDING_EVENTS);
        assert_eq!(
            client.pending_events.front(),
            Some(&json!({ "sequence": 2 }))
        );
        assert_eq!(
            client.pending_events.back(),
            Some(&json!({ "sequence": MAX_PENDING_EVENTS + 1 }))
        );
    }

    #[test]
    fn discards_pending_events_when_the_session_changes() {
        let (_directory, endpoint) = test_endpoint();
        let mut client = PetClient::new(endpoint);
        client.bind_session("session-1");
        client.enqueue(&[json!({ "type": "agent_started" })]);

        client.bind_session("session-2");

        assert!(client.pending_events.is_empty());
        assert_eq!(client.session_id.as_deref(), Some("session-2"));
    }

    fn test_endpoint() -> (Option<tempfile::TempDir>, PathBuf) {
        #[cfg(unix)]
        {
            let directory = tempfile::tempdir().expect("temporary directory");
            let endpoint = directory.path().join("codex-o-pet.sock");
            (Some(directory), endpoint)
        }
        #[cfg(windows)]
        {
            let sequence = NEXT_ENDPOINT.fetch_add(1, Ordering::Relaxed);
            (
                None,
                PathBuf::from(format!(
                    r"\\.\pipe\codex-o-pet-test-{}-{sequence}",
                    std::process::id()
                )),
            )
        }
    }
}
