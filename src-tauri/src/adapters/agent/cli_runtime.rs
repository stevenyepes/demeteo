use std::future::Future;
use std::io::{BufRead, BufReader, Read, Write};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::Mutex;

use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::domain::models::SessionInfo;
use crate::ports::agent_runtime::{
    AgentContext, AgentRuntime, AgentSession, AgentStartError, StderrHeartbeat,
};
use crate::ports::execution::InteractiveHandle;

/// Parse a single JSON-lines event from a CLI agent's stdout.
pub type EventParser = fn(line: &str) -> Option<AgentEvent>;

/// Construct command-line arguments for the CLI agent.
pub type ArgsBuilder = fn(ctx: &AgentContext, captured_session_id: Option<&str>) -> Vec<String>;

/// Shared runtime for one-shot CLI-based agents (opencode, hermes, claude, agy, etc.)
pub struct UnifiedCliRuntime {
    pub kind_str: &'static str,
    pub binary: &'static str,
    pub install_cmd: &'static str,
    pub parse_event: EventParser,
    pub build_args: ArgsBuilder,
}

impl AgentRuntime for UnifiedCliRuntime {
    fn kind(&self) -> &'static str {
        self.kind_str
    }

    fn is_available(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> bool {
        if machine_id == "local" || machine_id.is_empty() {
            super::is_binary_on_local_path(self.binary)
        } else {
            exec.run_command(
                machine_id,
                &format!("command -v {} >/dev/null 2>&1 && echo ok", self.binary),
            )
            .map(|out| out.trim() == "ok")
            .unwrap_or(false)
        }
    }

    fn install_command(&self) -> &'static str {
        self.install_cmd
    }

    fn start(
        &self,
        ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>>
    {
        let kind = self.kind_str;
        let parse_event = self.parse_event;
        let build_args = self.build_args;

        Box::pin(async move {
            let resolved_binary = if ctx.machine_id.is_empty() || ctx.machine_id == "local" {
                super::resolve_local_binary_path(&ctx.binary)
            } else {
                None
            };
            let session = UnifiedCliSession {
                session_id: format!("{}-{}", kind, ctx.thread_id),
                resolved_binary,
                ctx,
                parse_event,
                build_args,
                live_local: Mutex::new(None),
                live_remote: Mutex::new(None),
                captured_session_id: Arc::new(Mutex::new(None)),
                stderr_hb: StderrHeartbeat::new(),
            };
            Ok(Arc::new(session) as Arc<dyn AgentSession>)
        })
    }
}

#[allow(clippy::type_complexity)]
pub struct UnifiedCliSession {
    session_id: String,
    resolved_binary: Option<String>,
    ctx: AgentContext,
    parse_event: EventParser,
    build_args: ArgsBuilder,
    live_local: Mutex<Option<Arc<Mutex<std::process::Child>>>>,
    live_remote: Mutex<Option<Arc<Mutex<Box<dyn InteractiveHandle>>>>>,
    captured_session_id: Arc<Mutex<Option<String>>>,
    stderr_hb: StderrHeartbeat,
}

impl UnifiedCliSession {
    fn build_command(&self) -> Command {
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);
        let mut cmd = Command::new(binary);
        let captured = {
            if let Ok(guard) = self.captured_session_id.lock() {
                guard.clone()
            } else {
                None
            }
        };
        let args = (self.build_args)(&self.ctx, captured.as_deref());
        cmd.args(&args);
        cmd.current_dir(&self.ctx.cwd);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in &self.ctx.env {
            cmd.env(k, v);
        }
        cmd
    }

    fn spawn_local(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) {
        let mut cmd = self.build_command();
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::NotFound {
                    format!("binary not found at '{}'", binary)
                } else {
                    format!("failed to spawn {} ({}): {}", self.ctx.binary, binary, e)
                };
                let _ = tx.try_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: msg,
                    recoverable: false,
                });
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take();

        if let Some(mut stdin) = child.stdin.take() {
            let text_owned = text.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(text_owned.as_bytes());
                let _ = stdin.write_all(b"\n");
                let _ = stdin.flush();
            });
        }

        let child = Arc::new(Mutex::new(child));
        if let Ok(mut guard) = self.live_local.lock() {
            *guard = Some(child.clone());
        }

        if let Some(stderr) = stderr {
            let hb = self.stderr_hb.clone();
            let kind = self.ctx.binary.clone();
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        eprintln!("[{} stderr] {}", kind, trimmed);
                        hb.beat();
                    }
                }
            });
        }

        let exit_child = child.clone();
        let exit_code_fn = move || -> Option<i32> {
            exit_child
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten())
                .and_then(|status| status.code())
        };

        let session_capture = self.captured_session_id.clone();

        std::thread::spawn(move || {
            drain_lines(
                BufReader::new(stdout),
                parse_event,
                exit_code_fn,
                tx,
                Some(session_capture),
            );
        });
    }

    fn spawn_remote(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) {
        let captured = {
            if let Ok(guard) = self.captured_session_id.lock() {
                guard.clone()
            } else {
                None
            }
        };
        let args = (self.build_args)(&self.ctx, captured.as_deref());
        let machine_id = self.ctx.machine_id.clone();
        let binary = self.ctx.binary.clone();
        let cwd = self.ctx.cwd.clone();
        let env = self.ctx.env.clone();
        let exec = self.ctx.exec.clone();

        let handle = match exec.spawn_interactive(&machine_id, &binary, &args, &cwd, &env) {
            Ok(h) => h,
            Err(e) => {
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: format!("failed to spawn {} over SSH: {}", self.ctx.binary, e),
                    recoverable: false,
                });
                return;
            }
        };

        let _ = handle.write_line(text);

        let handle = Arc::new(Mutex::new(handle));
        if let Ok(mut guard) = self.live_remote.lock() {
            *guard = Some(handle.clone());
        }

        let exit_handle = handle.clone();
        let exit_code_fn = move || -> Option<i32> {
            exit_handle
                .lock()
                .ok()
                .and_then(|h| h.try_wait().ok().flatten())
        };

        let reader = HandleReader {
            handle: handle.clone(),
        };
        let session_capture = self.captured_session_id.clone();
        std::thread::spawn(move || {
            drain_lines(reader, parse_event, exit_code_fn, tx, Some(session_capture));
        });
    }

    fn kill_live_local(&self) {
        let child = match self.live_local.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(child) = child else { return };
        let Ok(mut c) = child.lock() else { return };
        match c.try_wait().ok().flatten() {
            Some(_) => {
                let _ = c.wait();
            }
            None => {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    fn kill_live_remote(&self) {
        let arc = match self.live_remote.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(arc) = arc else { return };
        let h = match arc.lock() {
            Ok(h) => h,
            Err(_) => return,
        };
        let _ = h.kill();
    }
}

impl AgentSession for UnifiedCliSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        self.kill_live_local();
        self.kill_live_remote();

        let parse_event = self.parse_event;
        let is_local = self.ctx.machine_id.is_empty() || self.ctx.machine_id == "local";
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);

        if is_local {
            self.spawn_local(text, parse_event, tx);
        } else {
            self.spawn_remote(text, parse_event, tx);
        }

        Box::pin(ReceiverStream::new(rx))
    }

    fn cancel(&self) -> Result<(), String> {
        self.kill()
    }

    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo::default()
    }

    fn kill(&self) -> Result<(), String> {
        self.kill_live_local();
        self.kill_live_remote();
        Ok(())
    }

    fn stderr_heartbeat(&self) -> Option<StderrHeartbeat> {
        Some(self.stderr_hb.clone())
    }
}

impl Drop for UnifiedCliSession {
    fn drop(&mut self) {
        self.kill_live_local();
        self.kill_live_remote();
    }
}

struct HandleReader {
    handle: Arc<Mutex<Box<dyn InteractiveHandle>>>,
}

impl Read for HandleReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let h = self.handle.lock().expect("HandleReader mutex poisoned");
        h.try_read(buf)
    }
}

fn drain_lines<R, F>(
    reader: R,
    parse_event: EventParser,
    exit_code_fn: F,
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
    session_capture: Option<Arc<Mutex<Option<String>>>>,
) where
    R: Read,
    F: FnOnce() -> Option<i32>,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut terminal = false;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(ref capture) = session_capture {
                    if let Ok(guard) = capture.lock() {
                        if guard.is_none() {
                            drop(guard);
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                let found_sid = v
                                    .get("sessionID")
                                    .or_else(|| v.get("session_id"))
                                    .or_else(|| v.get("conversationID"))
                                    .or_else(|| v.get("conversation_id"))
                                    .or_else(|| {
                                        v.get("data").and_then(|d| d.get("conversation_id"))
                                    })
                                    .or_else(|| v.get("data").and_then(|d| d.get("session_id")))
                                    .and_then(|s| s.as_str());
                                if let Some(sid) = found_sid {
                                    if let Ok(mut g) = capture.lock() {
                                        *g = Some(sid.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(evt) = parse_event(trimmed) {
                    let is_terminal = matches!(
                        evt,
                        AgentEvent::TurnComplete { .. } | AgentEvent::Error { .. }
                    );
                    if tx.blocking_send(evt).is_err() {
                        return;
                    }
                    if is_terminal {
                        terminal = true;
                        break;
                    }
                }
            }
        }
    }
    if !terminal {
        match exit_code_fn() {
            Some(0) | None => {
                let _ = tx.blocking_send(AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                });
            }
            Some(code) => {
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "agent_exit_nonzero".to_string(),
                    message: format!("agent exited with code {}", code),
                    recoverable: false,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tokio::sync::mpsc;

    fn mock_parse_event(line: &str) -> Option<AgentEvent> {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            return None;
        }
        let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;
        match v.get("type").and_then(|t| t.as_str()) {
            Some("text") => {
                let delta = v
                    .get("delta")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string();
                Some(AgentEvent::Text { delta })
            }
            Some("end_turn") => Some(AgentEvent::TurnComplete {
                stop_reason: StopReason::EndOfTurn,
            }),
            Some("error") => {
                let message = v
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("error")
                    .to_string();
                Some(AgentEvent::Error {
                    code: "cli_error".to_string(),
                    message,
                    recoverable: false,
                })
            }
            _ => None,
        }
    }

    fn run_drain<R, F>(reader: R, exit_code_fn: F) -> Vec<AgentEvent>
    where
        R: Read + Send + 'static,
        F: FnOnce() -> Option<i32> + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
        std::thread::spawn(move || {
            drain_lines(reader, mock_parse_event, exit_code_fn, tx, None);
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut events = Vec::new();
            while let Some(e) = rx.recv().await {
                events.push(e);
            }
            events
        })
    }

    #[test]
    fn drain_lines_reassembles_event_split_across_two_reads() {
        let full = br#"{"type":"text","delta":"hello world"}
{"type":"end_turn"}
"#;
        let split_at = 18;
        let (c1, c2) = full.split_at(split_at);
        let reader = Cursor::new(c1.to_vec()).chain(Cursor::new(c2.to_vec()));

        let events = run_drain(reader, || Some(0));
        assert_eq!(events.len(), 2, "got: {:?}", events);
        match &events[0] {
            AgentEvent::Text { delta } => assert_eq!(delta, "hello world"),
            e => panic!("expected Text, got {:?}", e),
        }
        match &events[1] {
            AgentEvent::TurnComplete { .. } => {}
            e => panic!("expected TurnComplete, got {:?}", e),
        }
    }

    #[test]
    fn drain_lines_handles_multiple_events_in_one_read() {
        let full = br#"{"type":"text","delta":"a"}
{"type":"text","delta":"b"}
{"type":"end_turn"}
"#;
        let events = run_drain(Cursor::new(full.to_vec()), || Some(0));
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "a"));
        assert!(matches!(&events[1], AgentEvent::Text { delta } if delta == "b"));
        assert!(matches!(&events[2], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_emits_error_on_nonzero_exit() {
        let reader = Cursor::new(br#"{"type":"text","delta":"x"}"#.to_vec());
        let events = run_drain(reader, || Some(137));
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "x"));
        match &events[1] {
            AgentEvent::Error { message, .. } => {
                assert!(
                    message.contains("137") || message.contains("nonzero"),
                    "got: {}",
                    message
                );
            }
            e => panic!("expected Error, got {:?}", e),
        }
    }

    #[test]
    fn drain_lines_emits_turn_complete_on_zero_exit_when_empty() {
        let events = run_drain(Cursor::new(Vec::new()), || Some(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_emits_error_when_empty_and_nonzero_exit() {
        let events = run_drain(Cursor::new(Vec::new()), || Some(1));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Error { message, .. } => {
                assert!(message.contains("1") || message.contains("nonzero"))
            }
            e => panic!("expected Error, got {:?}", e),
        }
    }

    #[test]
    fn drain_lines_skips_garbage_lines() {
        let full = b"this is not json\n{\"type\":\"end_turn\"}\n";
        let events = run_drain(Cursor::new(full.to_vec()), || Some(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_stops_at_terminal_event_even_if_more_data_pending() {
        let full = br#"{"type":"text","delta":"final"}
{"type":"end_turn"}
{"type":"text","delta":"this should be dropped"}
"#;
        let events = run_drain(Cursor::new(full.to_vec()), || Some(0));
        assert_eq!(events.len(), 2, "got: {:?}", events);
        assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "final"));
        assert!(matches!(&events[1], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_returns_early_when_consumer_drops() {
        let (tx, rx) = mpsc::channel::<AgentEvent>(1);
        drop(rx);
        let reader = Cursor::new(
            br#"{"type":"text","delta":"a"}
{"type":"text","delta":"b"}
{"type":"end_turn"}
"#
            .to_vec(),
        );
        drain_lines(reader, mock_parse_event, || Some(0), tx, None);
    }

    struct ChunkyHandle {
        chunks: std::sync::Mutex<Vec<Vec<u8>>>,
        exit_code: i32,
    }
    impl ChunkyHandle {
        fn new(chunks: Vec<&[u8]>, exit_code: i32) -> Self {
            Self {
                chunks: std::sync::Mutex::new(chunks.into_iter().map(<[u8]>::to_vec).collect()),
                exit_code,
            }
        }
    }
    impl InteractiveHandle for ChunkyHandle {
        fn write_line(&self, _: &str) -> std::io::Result<usize> {
            Ok(0)
        }
        fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut q = self.chunks.lock().unwrap();
            match q.first() {
                Some(chunk) => {
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    if n == chunk.len() {
                        q.remove(0);
                    } else {
                        q[0] = q[0].split_off(n);
                    }
                    Ok(n)
                }
                None => Ok(0),
            }
        }
        fn kill(&self) -> Result<(), String> {
            Ok(())
        }
        fn try_wait(&self) -> Result<Option<i32>, String> {
            Ok(Some(self.exit_code))
        }
    }

    #[test]
    fn handle_reader_reassembles_split_line_via_try_read() {
        let handle = Arc::new(Mutex::new(Box::new(ChunkyHandle::new(
            vec![
                b"{\"type\":\"text\",\"de",
                b"lta\":\"split\"}\n",
                b"{\"type\":\"end_turn\"}\n",
            ],
            0,
        )) as Box<dyn InteractiveHandle>));
        let handle_for_exit = handle.clone();
        let reader = HandleReader { handle };
        let events = run_drain(reader, move || {
            handle_for_exit
                .lock()
                .ok()
                .and_then(|h| h.try_wait().ok().flatten())
        });
        assert_eq!(events.len(), 2, "got: {:?}", events);
        match &events[0] {
            AgentEvent::Text { delta } => assert_eq!(delta, "split"),
            e => panic!("expected Text, got {:?}", e),
        }
        assert!(matches!(&events[1], AgentEvent::TurnComplete { .. }));
    }
}
