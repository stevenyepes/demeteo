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
            usage: None,
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
        drain_lines(reader, mock_parse_event, exit_code_fn, tx, None, None);
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
    drain_lines(reader, mock_parse_event, || Some(0), tx, None, None);
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

/// Records the `ShellOptions` (and command) handed to `run_command_with`
/// so the availability-probe test can assert it runs under a login shell.
/// Every other port method is an inert stub — the probe only ever calls
/// `run_command`/`run_command_with`.
struct ShellOptsRecorder {
    last_opts: std::sync::Mutex<Option<crate::ports::execution::ShellOptions>>,
    last_cmd: std::sync::Mutex<Option<String>>,
}

impl ShellOptsRecorder {
    fn new() -> Self {
        Self {
            last_opts: std::sync::Mutex::new(None),
            last_cmd: std::sync::Mutex::new(None),
        }
    }
}

#[async_trait::async_trait]
impl crate::ports::execution::ExecutionPort for ShellOptsRecorder {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    // Deliberately DO NOT override `run_command`: its trait default routes
    // through `run_command_with(.., ShellOptions::default())`, so a
    // regression that reverts the probe to the bare non-login `run_command`
    // still lands here — and records `login_shell: false`, failing the test.
    async fn run_command_with(
        &self,
        _: &str,
        cmd: &str,
        opts: crate::ports::execution::ShellOptions,
    ) -> Result<String, String> {
        *self.last_cmd.lock().unwrap() = Some(cmd.to_string());
        *self.last_opts.lock().unwrap() = Some(opts);
        // Non-empty stdout other than "ok" would make the probe report
        // unavailable; echo the sentinel the probe greps for.
        Ok("ok".to_string())
    }
    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    async fn get_metadata(
        &self,
        _: &str,
        path: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        Ok(crate::ports::execution::SftpEntry {
            name: path.into(),
            path: path.into(),
            is_dir: false,
            size: 0,
            modified: 0,
        })
    }
    async fn list_dir(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Ok(vec![])
    }
    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        Ok("/tmp".to_string())
    }
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Ok("stub".to_string())
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("unsupported".to_string())
    }
    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        Err("stub".to_string())
    }
}

fn probe_build_args(_: &AgentContext, _: Option<&str>, _: &str) -> Vec<String> {
    vec![]
}

fn probe_perm_env(
    _: &crate::domain::permission::PermissionProfile,
) -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

/// A remote availability probe must run under a **login** shell so the
/// target user's profile is sourced (`PATH` additions from
/// `~/.profile`/`~/.bashrc`, `mise`/`asdf` shims, installer dirs like
/// opencode's `~/.opencode/bin`). A bare non-login `command -v` misses all
/// of those and reports a correctly-installed agent as "Missing".
#[test]
fn remote_availability_probe_uses_a_login_shell() {
    let runtime = UnifiedCliRuntime {
        kind_str: "opencode",
        binary: "opencode",
        install_cmd: "curl -fsSL https://opencode.ai/install | bash",
        parse_event: mock_parse_event,
        build_args: probe_build_args,
        perm_env: probe_perm_env,
    };
    let rec = ShellOptsRecorder::new();

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let available = tokio_rt.block_on(runtime.is_available(&rec, "demeteo-remote"));

    assert!(available, "probe returning 'ok' should report the agent available");

    let opts = rec
        .last_opts
        .lock()
        .unwrap()
        .clone()
        .expect("remote probe must go through run_command_with");
    assert!(
        opts.login_shell,
        "remote availability probe must request a login shell so the profile PATH is sourced"
    );
    assert!(
        opts.interactive,
        "remote availability probe must be interactive so ~/.bashrc (mise/asdf/nvm tool \
         activation) is sourced — matching how the agent is actually spawned"
    );

    let cmd = rec.last_cmd.lock().unwrap().clone().unwrap_or_default();
    assert!(
        cmd.contains("command -v opencode"),
        "probe should resolve the agent binary, got: {}",
        cmd
    );
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
