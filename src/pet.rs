use std::{
    collections::{HashMap, VecDeque},
    env, io,
    io::Write,
    path::{Path, PathBuf},
};

use interprocess::local_socket::{GenericFilePath, Stream, ToFsName as _, traits::Stream as _};
use serde_json::{Value, json};

use crate::mcp::PetTransport;

const ENDPOINT_ENV: &str = "O_PET_ENDPOINT";
const CLIENT_ID: &str = "codex-o-pet";
const MAX_PENDING_EVENTS: usize = 256;
const MAX_SESSIONS: usize = 16;

pub struct PetClient {
    endpoint: PathBuf,
    sessions: HashMap<String, PetSession>,
    use_order: u64,
}

struct PetSession {
    stream: Option<Stream>,
    pending_events: VecDeque<Value>,
    last_used: u64,
}

impl PetClient {
    pub fn from_environment() -> io::Result<Self> {
        Ok(Self::new(resolve_endpoint()?))
    }

    pub fn new(endpoint: PathBuf) -> Self {
        Self {
            endpoint,
            sessions: HashMap::new(),
            use_order: 0,
        }
    }

    fn touch_session(&mut self, session_id: &str) {
        self.use_order = self.use_order.saturating_add(1);
        if let Some(session) = self.sessions.get_mut(session_id) {
            session.last_used = self.use_order;
            return;
        }

        if self.sessions.len() >= MAX_SESSIONS {
            self.evict_oldest_session();
        }
        self.sessions.insert(
            session_id.to_string(),
            PetSession {
                stream: None,
                pending_events: VecDeque::new(),
                last_used: self.use_order,
            },
        );
    }

    fn evict_oldest_session(&mut self) {
        let oldest_session_id = self
            .sessions
            .iter()
            .min_by_key(|(_, session)| session.last_used)
            .map(|(session_id, _)| session_id.clone())
            .expect("a full session pool cannot be empty");
        let mut session = self
            .sessions
            .remove(&oldest_session_id)
            .expect("the selected session must exist");
        session.close();
    }

    fn close_all(&mut self) {
        for session in self.sessions.values_mut() {
            session.close();
        }
    }

    #[cfg(test)]
    fn shutdown(&mut self) {
        self.close_all();
    }
}

impl PetSession {
    fn enqueue(&mut self, events: &[Value]) {
        self.pending_events.extend(events.iter().cloned());
        let overflow = self.pending_events.len().saturating_sub(MAX_PENDING_EVENTS);
        self.pending_events.drain(..overflow);
    }

    fn connect(&mut self, endpoint: &Path, session_id: &str) -> io::Result<()> {
        let name = endpoint.as_os_str().to_fs_name::<GenericFilePath>()?;
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

    fn connect_and_flush(&mut self, endpoint: &Path, session_id: &str) -> io::Result<()> {
        if self.stream.is_none() {
            self.connect(endpoint, session_id)?;
        }
        self.flush_pending()
    }

    fn close(&mut self) {
        if let Some(mut stream) = self.stream.take() {
            let _ = write_line(&mut stream, &json!({ "type": "goodbye" }));
        }
    }
}

impl PetTransport for PetClient {
    fn deliver(&mut self, session_id: &str, events: &[Value]) -> io::Result<()> {
        self.touch_session(session_id);
        let endpoint = self.endpoint.as_path();
        let session = self
            .sessions
            .get_mut(session_id)
            .expect("a touched session must exist");
        session.enqueue(events);
        if let Err(first_error) = session.connect_and_flush(endpoint, session_id) {
            session.stream = None;
            session
                .connect_and_flush(endpoint, session_id)
                .map_err(|retry_error| {
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
        self.close_all();
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
        client.touch_session("session-1");

        let events = (0..MAX_PENDING_EVENTS + 2)
            .map(|sequence| json!({ "sequence": sequence }))
            .collect::<Vec<_>>();
        let session = client
            .sessions
            .get_mut("session-1")
            .expect("session exists");
        session.enqueue(&events);

        assert_eq!(session.pending_events.len(), MAX_PENDING_EVENTS);
        assert_eq!(
            session.pending_events.front(),
            Some(&json!({ "sequence": 2 }))
        );
        assert_eq!(
            session.pending_events.back(),
            Some(&json!({ "sequence": MAX_PENDING_EVENTS + 1 }))
        );
    }

    #[test]
    fn retains_independent_pending_queues_for_multiple_sessions() {
        let (_directory, endpoint) = test_endpoint();
        let mut client = PetClient::new(endpoint);
        client.touch_session("session-1");
        client
            .sessions
            .get_mut("session-1")
            .expect("first session")
            .enqueue(&[json!({ "type": "agent_started" })]);
        client.touch_session("session-2");
        client
            .sessions
            .get_mut("session-2")
            .expect("second session")
            .enqueue(&[json!({ "type": "turn_started" })]);

        assert_eq!(client.sessions.len(), 2);
        assert_eq!(
            client.sessions["session-1"].pending_events.front(),
            Some(&json!({ "type": "agent_started" }))
        );
        assert_eq!(
            client.sessions["session-2"].pending_events.front(),
            Some(&json!({ "type": "turn_started" }))
        );
    }

    #[test]
    fn evicts_the_least_recently_used_session_when_the_pool_is_full() {
        let (_directory, endpoint) = test_endpoint();
        let mut client = PetClient::new(endpoint);
        for sequence in 0..MAX_SESSIONS {
            client.touch_session(&format!("session-{sequence}"));
        }

        client.touch_session("session-0");
        client.touch_session("session-new");

        assert_eq!(client.sessions.len(), MAX_SESSIONS);
        assert!(client.sessions.contains_key("session-0"));
        assert!(!client.sessions.contains_key("session-1"));
        assert!(client.sessions.contains_key("session-new"));
    }

    #[test]
    fn keeps_real_connections_open_for_interleaved_sessions() {
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
            let mut streams = (0..2)
                .map(|_| listener.accept().expect("accept bridge connection"))
                .collect::<Vec<_>>();
            streams
                .iter_mut()
                .map(|stream| {
                    let mut text = String::new();
                    stream
                        .read_to_string(&mut text)
                        .expect("read bridge messages");
                    text
                })
                .collect::<Vec<_>>()
        });

        let mut client = PetClient::new(endpoint);
        client
            .deliver("session-1", &[json!({ "type": "agent_started" })])
            .expect("deliver first session event");
        client
            .deliver("session-2", &[json!({ "type": "agent_started" })])
            .expect("deliver second session event");
        client
            .deliver("session-1", &[json!({ "type": "turn_started" })])
            .expect("reuse first session connection");
        client.shutdown();

        let messages = reader
            .join()
            .expect("reader thread")
            .into_iter()
            .map(|text| {
                text.lines()
                    .map(|line| serde_json::from_str::<Value>(line).expect("valid JSON line"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        assert_eq!(messages[0][0]["sessionId"], "session-1");
        assert_eq!(messages[0][1]["event"]["type"], "agent_started");
        assert_eq!(messages[0][2]["event"]["type"], "turn_started");
        assert_eq!(messages[0][3]["type"], "goodbye");
        assert_eq!(messages[1][0]["sessionId"], "session-2");
        assert_eq!(messages[1][1]["event"]["type"], "agent_started");
        assert_eq!(messages[1][2]["type"], "goodbye");
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
