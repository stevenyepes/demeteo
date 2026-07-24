use std::io::{Read, Write};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use tauri::ipc::{Channel, InvokeResponseBody};

use super::{ActiveSession, Broadcast, ReadSource, SessionState, WriteSink};

pub const DRAIN_WAIT_MS: u64 = 2_000;

pub fn broadcast_with(channels: Vec<Channel<Vec<u8>>>) -> Arc<Mutex<Broadcast>> {
    let mut broadcast = Broadcast::new();
    broadcast.channels = channels;
    Arc::new(Mutex::new(broadcast))
}

pub fn channel_count(broadcast: &Arc<Mutex<Broadcast>>) -> usize {
    broadcast.lock().expect("broadcast lock").channels.len()
}

pub fn capturing_channel() -> (Channel<Vec<u8>>, Arc<Mutex<Vec<u8>>>) {
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json_str) = body {
            if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(&json_str) {
                *captured_clone.lock().expect("capture lock") = bytes;
            }
        }
        Ok(())
    });
    (channel, captured)
}

pub fn appending_capturing_channel() -> (Channel<Vec<u8>>, Arc<Mutex<Vec<u8>>>) {
    let captured: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let captured_clone = captured.clone();
    let channel = Channel::new(move |body: InvokeResponseBody| {
        if let InvokeResponseBody::Json(json_str) = body {
            if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(&json_str) {
                captured_clone
                    .lock()
                    .expect("capture lock")
                    .extend_from_slice(&bytes);
            }
        }
        Ok(())
    });
    (channel, captured)
}

pub fn dead_channel() -> Channel<Vec<u8>> {
    Channel::new(|_body| -> tauri::Result<()> { Err(tauri::Error::FailedToReceiveMessage) })
}

pub fn wait_for(timeout_ms: u64, predicate: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    predicate()
}

pub fn wait_until(predicate: impl Fn() -> bool) -> bool {
    wait_for(DRAIN_WAIT_MS, predicate)
}

pub struct TestSessionHandles {
    pub broadcast: Arc<Mutex<Broadcast>>,
    pub writer: Option<Arc<Mutex<Box<dyn Write + Send>>>>,
    pub drain: Option<JoinHandle<()>>,
}

pub struct TestSessionBuilder {
    agent: Option<String>,
    activity_nonce: Option<String>,
    connected: bool,
    scrollback_seed: Vec<u8>,
    live_drain: bool,
}

impl Default for TestSessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl TestSessionBuilder {
    pub fn new() -> Self {
        Self {
            agent: None,
            activity_nonce: None,
            connected: true,
            scrollback_seed: Vec::new(),
            live_drain: false,
        }
    }

    pub fn disconnected(mut self) -> Self {
        self.connected = false;
        self
    }

    pub fn agent(mut self, kind: &str) -> Self {
        self.agent = Some(kind.to_string());
        self
    }

    pub fn activity_nonce(mut self, nonce: &str) -> Self {
        self.activity_nonce = Some(nonce.to_string());
        self
    }

    pub fn seed_scrollback(mut self, seed: &[u8]) -> Self {
        self.scrollback_seed = seed.to_vec();
        self
    }

    pub fn live_drain(mut self) -> Self {
        self.live_drain = true;
        self
    }

    pub fn build(self) -> (ActiveSession, TestSessionHandles) {
        let broadcast: Arc<Mutex<Broadcast>> = Arc::new(Mutex::new(Broadcast::new()));
        let frontend_channel = broadcast.clone();
        if !self.scrollback_seed.is_empty() {
            super::send_chunk(&frontend_channel, self.scrollback_seed);
        }
        let (read_source, write_sink, keepalive, _child_pid, _settings) =
            super::start_local_pty("local", &None, &None, 80, 24).expect("start_local_pty");

        let (writer, drain) = if self.live_drain {
            let reader = match &read_source {
                ReadSource::LocalPty(r) => r.clone(),
                ReadSource::Ssh(_) => unreachable!("local pty path"),
            };
            let writer_handle: Arc<Mutex<Box<dyn Write + Send>>> = match &write_sink {
                WriteSink::LocalPty(w) => w.clone(),
                WriteSink::Ssh(_) => unreachable!("local pty path"),
            };
            let keepalive_for_thread = keepalive.clone();
            let frontend_channel_for_thread = frontend_channel.clone();
            let drain_handle = thread::spawn(move || {
                let mut buffer = [0u8; 8192];
                loop {
                    let read_result = reader.lock().expect("reader lock").read(&mut buffer);
                    match read_result {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = buffer[..n].to_vec();
                            super::send_chunk(&frontend_channel_for_thread, chunk);
                        }
                    }
                }
                drop(keepalive_for_thread);
            });
            (Some(writer_handle), Some(drain_handle))
        } else {
            (None, None)
        };

        let session = ActiveSession {
            read_source,
            write_sink,
            _keepalive: keepalive,
            machine_id: "local".to_string(),
            machine_name: "local".to_string(),
            created_at: 0,
            child_pid: None,
            agent: Mutex::new(self.agent),
            activity_nonce: self.activity_nonce,
            activity_settings_path: None,
            last_output_at: Arc::new(Mutex::new(Instant::now())),
            frontend_channel,
            display_title: Mutex::new(None),
            work_dir: None,
            work_branch: None,
            connected: Arc::new(AtomicBool::new(self.connected)),
        };

        (
            session,
            TestSessionHandles {
                broadcast,
                writer,
                drain,
            },
        )
    }

    pub fn install(self, state: &SessionState, id: &str) -> TestSessionHandles {
        let (session, handles) = self.build();
        state
            .sessions
            .lock()
            .expect("sessions lock")
            .insert(id.to_string(), session);
        handles
    }
}

pub fn local_machine() -> crate::domain::models::Machine {
    crate::infrastructure::worktree::machine_resolver::local_machine()
}

pub fn activity_last_emitted(state: &SessionState, id: &str) -> Option<String> {
    state
        .activity
        .lock()
        .expect("activity lock")
        .get(id)
        .and_then(|sa| sa.last_emitted.clone())
}

pub fn shell_single_unquote(quoted: &str) -> String {
    let inner = quoted
        .strip_prefix('\'')
        .and_then(|s| s.strip_suffix('\''))
        .expect("value must be wrapped in single quotes");
    inner.replace("'\\''", "'")
}
