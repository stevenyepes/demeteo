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
                usage: None,
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

/// A reader that yields some data, then fails with a read error — the shape
/// of the remote SSH stream when the transport drops mid-turn.
struct FailingReader {
    data: Cursor<Vec<u8>>,
    errored: bool,
}

impl Read for FailingReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self.data.read(buf) {
            Ok(0) if !self.errored => {
                self.errored = true;
                Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Timed out waiting on socket",
                ))
            }
            other => other,
        }
    }
}

#[test]
fn drain_lines_reports_stream_loss_when_read_errors_and_agent_still_running() {
    // Regression: the remote agent's SSH stream broke mid-turn while the
    // process was still running (`exit_code_fn` → None). The old behaviour
    // fabricated a clean TurnComplete, so a half-finished agent looked like
    // a successful step with a mysteriously missing artifact.
    let reader = FailingReader {
        data: Cursor::new(b"{\"type\":\"text\",\"delta\":\"partial\"}\n".to_vec()),
        errored: false,
    };
    let events = run_drain(reader, || None);
    assert_eq!(events.len(), 2, "got: {:?}", events);
    assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "partial"));
    match &events[1] {
        AgentEvent::Error { code, message, .. } => {
            assert_eq!(code, "agent_stream_lost");
            assert!(message.contains("Timed out"), "got: {}", message);
        }
        e => panic!("expected agent_stream_lost Error, got {:?}", e),
    }
}

#[test]
fn drain_lines_still_completes_on_clean_eof_without_exit_status() {
    // Clean EOF + no exit status yet (local child closed stdout but hasn't
    // been reaped) stays a normal turn completion.
    let reader = Cursor::new(br#"{"type":"text","delta":"done"}"#.to_vec());
    let events = run_drain(reader, || None);
    assert_eq!(events.len(), 2, "got: {:?}", events);
    assert!(matches!(&events[1], AgentEvent::TurnComplete { .. }));
}

/// Live end-to-end check of the remote agent stream through the REAL
/// production wiring: `SshClientAdapter::spawn_interactive` (PTY, login
/// shell, 10s session timeout, 30s keepalive) → `HandleReader` →
/// `drain_lines`. The fake agent's output crosses both historical
/// killers: the ~30s keepalive-due read abort and a >10s silent gap.
///
/// Ignored by default (needs a reachable machine). Run with:
/// `DEMETEO_SSH_PROBE=<host>,<user>,<key_path> cargo test -p demeteo-core \
///  remote_pty_stream_survives -- --ignored --nocapture`
#[test]
#[ignore]
fn remote_pty_stream_survives_keepalive_and_silence_live() {
    use crate::domain::models::Machine;
    use crate::ports::execution::ExecutionPort;

    let Ok(spec) = std::env::var("DEMETEO_SSH_PROBE") else {
        panic!("set DEMETEO_SSH_PROBE=<host>,<user>,<key_path>");
    };
    let mut it = spec.splitn(3, ',');
    let (host, user, key) = (
        it.next().unwrap().to_string(),
        it.next().expect("user").to_string(),
        it.next().expect("key path").to_string(),
    );

    struct OneMachine(Machine);
    impl crate::ports::db::MachineRepository for OneMachine {
        fn get_machines(&self) -> Result<Vec<Machine>, String> {
            Ok(vec![self.0.clone()])
        }
        fn get_machine(
            &self,
            id: &crate::domain::ids::MachineId,
        ) -> Result<Option<Machine>, String> {
            Ok((id.0 == self.0.id.0).then(|| self.0.clone()))
        }
        fn add(&self, _: Machine) -> Result<(), String> {
            unimplemented!()
        }
        fn update(&self, _: Machine) -> Result<(), String> {
            unimplemented!()
        }
        fn delete(&self, _: &crate::domain::ids::MachineId) -> Result<(), String> {
            unimplemented!()
        }
        fn get_agent_profiles(
            &self,
            _: &crate::domain::ids::MachineId,
        ) -> Result<Vec<crate::domain::models::AgentProfile>, String> {
            Ok(vec![])
        }
        fn add_agent_profile(&self, _: crate::domain::models::AgentProfile) -> Result<(), String> {
            unimplemented!()
        }
        fn delete_agent_profile(
            &self,
            _: &crate::domain::ids::AgentProfileId,
        ) -> Result<(), String> {
            unimplemented!()
        }
    }

    let machine = Machine {
        id: crate::domain::ids::MachineId("m-probe".into()),
        name: "probe".into(),
        host,
        port: 22,
        username: user,
        auth_type: "key".into(),
        key_path: Some(key),
        agents: None,
        auto_approved_rules: None,
        use_login_shell: Some(true),
        setup_commands: None,
        notify_webhook_url: None,
    };
    let adapter = crate::adapters::ssh::client::SshClientAdapter::new(std::sync::Arc::new(
        OneMachine(machine),
    ));

    // Fake agent: JSON burst every 3s for 36s (crosses the 30s keepalive),
    // then 15s of total silence (crosses the 10s session timeout), then a
    // final text + end_turn.
    let script = r#"for i in $(seq 1 12); do echo "{\"type\":\"text\",\"delta\":\"tick-$i\"}"; sleep 3; done; sleep 15; echo '{"type":"text","delta":"after-silence"}'; echo '{"type":"end_turn"}'"#;
    let handle = adapter
        .spawn_interactive(
            "m-probe",
            "bash",
            &["-c".to_string(), script.to_string()],
            "/tmp",
            &std::collections::HashMap::new(),
        )
        .expect("spawn_interactive");

    let handle = std::sync::Arc::new(std::sync::Mutex::new(handle));
    let exit_handle = handle.clone();
    let reader = HandleReader { handle };
    let start = std::time::Instant::now();
    let events = run_drain(reader, move || {
        exit_handle
            .lock()
            .ok()
            .and_then(|h| h.try_wait().ok().flatten())
    });
    let elapsed = start.elapsed();

    let texts: Vec<&AgentEvent> = events
        .iter()
        .filter(|e| matches!(e, AgentEvent::Text { .. }))
        .collect();
    assert!(
        elapsed.as_secs() >= 50,
        "stream ended early at {:?} — transport truncation regressed. events: {:?}",
        elapsed,
        events
    );
    assert_eq!(texts.len(), 13, "missing text events: {:?}", events);
    assert!(
        matches!(events.last(), Some(AgentEvent::TurnComplete { .. })),
        "expected trailing TurnComplete, got: {:?}",
        events.last()
    );
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
        effort_env: no_effort_env,
        display_label: "OpenCode",
        lists_models: true,
        default_model: None,
        effort_levels: &[],
    };
    let rec = ShellOptsRecorder::new();

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let available = tokio_rt.block_on(runtime.is_available(&rec, "demeteo-remote"));

    assert!(
        available,
        "probe returning 'ok' should report the agent available"
    );

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

#[test]
fn no_effort_env_injects_nothing_at_any_level() {
    // The `effort_env` translator for every agent that carries effort on
    // argv (codex, opencode) or not at all (hermes).
    assert!(no_effort_env(None).is_empty());
    for level in EffortLevel::ALL {
        assert!(no_effort_env(Some(level)).is_empty());
    }
}
