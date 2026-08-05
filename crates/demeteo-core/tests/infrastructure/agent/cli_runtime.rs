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
        drain_lines(
            reader,
            mock_parse_event,
            exit_code_fn,
            tx,
            None,
            None,
            "stub-agent".to_string(),
        );
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
fn an_agent_that_exits_zero_without_writing_is_not_a_completed_turn() {
    // The documented Windows signature of these CLIs, and what a `.cmd` shim
    // does when its interpreter is gone. Reported as a completion it becomes a
    // green turn that merely produced no deliverable — a verdict fabricated
    // rather than measured, in a gated orchestrator.
    let events = run_drain(Cursor::new(Vec::new()), || Some(0));
    assert_eq!(events.len(), 1, "got: {:?}", events);
    match &events[0] {
        AgentEvent::Error { code, message, .. } => {
            assert_eq!(code, "agent_no_output");
            assert!(message.contains("stub-agent"), "got: {}", message);
        }
        e => panic!("expected an agent_no_output Error, got {:?}", e),
    }
}

#[test]
fn a_process_that_never_reached_a_verdict_is_never_routed_back_as_feedback() {
    // The turn loop reads this to decide between `Environmental` and `Failed`.
    // Every ending below tested nothing, so re-implementing the code cannot
    // close it; only `cli_error` — the agent's own report — is feedback.
    for ending in [
        TurnEnding::StreamLost,
        TurnEnding::NonZeroExit(1),
        TurnEnding::NoOutput,
    ] {
        let code = ending.error_code().expect("not a turn");
        assert!(is_process_level_error(code), "{:?} → {}", ending, code);
    }
    assert_eq!(TurnEnding::Complete.error_code(), None);
    assert!(is_process_level_error("spawn_failed"));
    assert!(!is_process_level_error("cli_error"));
}

#[test]
fn a_clean_exit_after_output_stays_a_turn_and_an_empty_one_does_not() {
    assert_eq!(
        classify_turn_ending(Some(0), false, true),
        TurnEnding::Complete
    );
    assert_eq!(
        classify_turn_ending(Some(0), false, false),
        TurnEnding::NoOutput
    );
    // Not yet reaped: same two answers, so a slow `wait` cannot flip a silent
    // agent into a success.
    assert_eq!(
        classify_turn_ending(None, false, true),
        TurnEnding::Complete
    );
    assert_eq!(
        classify_turn_ending(None, false, false),
        TurnEnding::NoOutput
    );
}

#[test]
fn a_lost_stream_and_a_nonzero_exit_still_outrank_the_silence_check() {
    // Both were already their own ending and must not be re-read as "no
    // output": a broken stream is a lost transport, and a non-zero exit is the
    // one ending that *is* the process's own verdict.
    assert_eq!(
        classify_turn_ending(None, true, false),
        TurnEnding::StreamLost
    );
    assert_eq!(
        classify_turn_ending(Some(137), false, false),
        TurnEnding::NonZeroExit(137)
    );
    assert_eq!(
        classify_turn_ending(Some(137), true, true),
        TurnEnding::NonZeroExit(137)
    );
    // A read error alongside a clean exit is not a lost stream — the process
    // finished — so the output check decides, exactly as it did before.
    assert_eq!(
        classify_turn_ending(Some(0), true, true),
        TurnEnding::Complete
    );
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

/// Like [`mock_parse_event`], but `warning` lines produce a **recoverable**
/// error — codex's shape, where a runtime warning has no channel of its own
/// and rides the same non-fatal error item as a real per-item failure.
fn mock_parse_event_with_warnings(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("warning") {
        return mock_parse_event(line);
    }
    Some(AgentEvent::Error {
        code: "item_error".to_string(),
        message: v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("warning")
            .to_string(),
        recoverable: true,
        usage: None,
    })
}

#[test]
fn drain_lines_keeps_reading_past_a_recoverable_error() {
    // A codex turn that hit its own backpressure mid-flight. The warning is
    // surfaced, but the turn is still running: truncating here dropped the
    // real result and reported "Resolver failed. in-process app-server event
    // stream lagged; dropped 13 events" while the agent was still working.
    let full = br#"{"type":"text","delta":"before"}
{"type":"warning","message":"in-process app-server event stream lagged; dropped 13 events"}
{"type":"text","delta":"after"}
{"type":"end_turn"}
"#;
    let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
    std::thread::spawn(move || {
        drain_lines(
            Cursor::new(full.to_vec()),
            mock_parse_event_with_warnings,
            || Some(0),
            tx,
            None,
            None,
            "stub-agent".to_string(),
        );
    });
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let events = rt.block_on(async {
        let mut events = Vec::new();
        while let Some(e) = rx.recv().await {
            events.push(e);
        }
        events
    });

    assert_eq!(events.len(), 4, "got: {:?}", events);
    assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "before"));
    assert!(matches!(&events[1], AgentEvent::Error { recoverable, .. } if *recoverable));
    assert!(
        matches!(&events[2], AgentEvent::Text { delta } if delta == "after"),
        "everything after the warning was truncated: {:?}",
        events
    );
    assert!(matches!(&events[3], AgentEvent::TurnComplete { .. }));
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
    drain_lines(
        reader,
        mock_parse_event,
        || Some(0),
        tx,
        None,
        None,
        "stub-agent".to_string(),
    );
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
        model_listing: Some(crate::ports::agent_runtime::ModelListing::MODELS_SUBCOMMAND),
        default_model: None,
        static_env: &[],
        effort_levels: &[],
    };
    let rec = ShellOptsRecorder::new();

    let tokio_rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let available = tokio_rt.block_on(runtime.availability(&rec, "demeteo-remote"));

    assert_eq!(
        available,
        crate::domain::models::Availability::Installed,
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

/// A parser that reports token usage, for the cumulative-footprint tests.
/// `usage` lines emit a mid-turn `Usage` snapshot; `end_turn` lines emit the
/// terminal `TurnComplete` with the same fields.
fn mock_parse_usage_event(line: &str) -> Option<AgentEvent> {
    let v: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let usage = crate::domain::agent_event::Usage {
        input_tokens: v.get("input").and_then(|t| t.as_u64()).unwrap_or(0),
        output_tokens: v.get("output").and_then(|t| t.as_u64()).unwrap_or(0),
        cost_usd: None,
        cache_read_input_tokens: v.get("cache_read").and_then(|t| t.as_u64()).unwrap_or(0),
        cache_creation_input_tokens: v
            .get("cache_creation")
            .and_then(|t| t.as_u64())
            .unwrap_or(0),
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("usage") => Some(AgentEvent::Usage(usage)),
        Some("usage_delta") => Some(AgentEvent::UsageDelta(usage)),
        Some("end_turn") => Some(AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: Some(usage),
        }),
        _ => None,
    }
}

#[test]
fn drain_lines_counts_cache_tokens_in_cumulative_footprint() {
    // A warm resumed Claude Code session reports almost its entire context
    // as `cache_read_input_tokens` — `input_tokens` is only the uncached
    // remainder. The context-window watchdog compares this counter against
    // the model's context window, so cache tokens must count; summing only
    // input+output made the watchdog fire late or never.
    let cumulative = Arc::new(AtomicU64::new(0));
    let input = concat!(
        r#"{"type":"usage","input":100,"output":50,"cache_read":80000,"cache_creation":2000}"#,
        "\n",
        // The terminal snapshot is *smaller* — the high-water mark must hold.
        r#"{"type":"end_turn","input":90,"output":40,"cache_read":70000,"cache_creation":1000}"#,
        "\n",
    );
    let (tx, mut rx) = mpsc::channel::<AgentEvent>(8);
    let cum = cumulative.clone();
    let handle = std::thread::spawn(move || {
        drain_lines(
            Cursor::new(input),
            mock_parse_usage_event,
            || Some(0),
            tx,
            None,
            Some(cum),
            "stub-agent".to_string(),
        );
    });
    while rx.blocking_recv().is_some() {}
    handle.join().unwrap();
    assert_eq!(
        cumulative.load(Ordering::Relaxed),
        100 + 50 + 80_000 + 2_000,
        "footprint must be input + output + cache_read + cache_creation, monotonic-max"
    );
}

#[test]
fn drain_lines_takes_the_largest_usage_delta_not_their_sum() {
    let cumulative = Arc::new(AtomicU64::new(0));
    let input = concat!(
        r#"{"type":"usage_delta","input":1000,"output":50}"#,
        "\n",
        r#"{"type":"usage_delta","input":400,"output":20}"#,
        "\n",
        r#"{"type":"usage_delta","input":700,"output":30}"#,
        "\n",
    );
    let (tx, mut rx) = mpsc::channel::<AgentEvent>(8);
    let cum = cumulative.clone();
    let handle = std::thread::spawn(move || {
        drain_lines(
            Cursor::new(input),
            mock_parse_usage_event,
            || Some(0),
            tx,
            None,
            Some(cum),
            "stub-agent".to_string(),
        );
    });
    while rx.blocking_recv().is_some() {}
    handle.join().unwrap();
    assert_eq!(cumulative.load(Ordering::Relaxed), 1050);
}

fn captured_session_id(input: &'static str) -> Option<String> {
    let capture = Arc::new(Mutex::new(None));
    let (tx, mut rx) = mpsc::channel::<AgentEvent>(8);
    let cap = capture.clone();
    let handle = std::thread::spawn(move || {
        drain_lines(
            Cursor::new(input),
            mock_parse_event,
            || Some(0),
            tx,
            Some(cap),
            None,
            "stub-agent".to_string(),
        );
    });
    while rx.blocking_recv().is_some() {}
    handle.join().unwrap();
    let guard = capture.lock().unwrap();
    guard.clone()
}

#[test]
fn a_bare_id_is_captured_only_from_a_session_typed_line() {
    // pi's header names the session id `id`. Everything else on the stream
    // names *its own* id the same way, and the capture takes the first
    // match — so the tool line has to lose to the header that follows it.
    let sid = captured_session_id(concat!(
        r#"{"type":"tool_execution_start","id":"tool-abc","toolName":"read"}"#,
        "\n",
        r#"{"type":"session","version":3,"id":"019fbb89-real"}"#,
        "\n",
        r#"{"type":"end_turn"}"#,
        "\n",
    ));
    assert_eq!(sid.as_deref(), Some("019fbb89-real"));
}

#[test]
fn a_bare_id_on_a_non_session_line_is_never_captured() {
    let sid = captured_session_id(concat!(
        r#"{"type":"message_start","id":"msg-1"}"#,
        "\n",
        r#"{"type":"tool_execution_end","id":"tool-abc","isError":false}"#,
        "\n",
        r#"{"type":"end_turn"}"#,
        "\n",
    ));
    assert_eq!(sid, None);
}

#[test]
fn the_documented_session_keys_still_win_over_a_bare_id() {
    let sid = captured_session_id(concat!(
        r#"{"type":"init","sessionID":"ses_opencode","id":"not-this-one"}"#,
        "\n",
        r#"{"type":"end_turn"}"#,
        "\n",
    ));
    assert_eq!(sid.as_deref(), Some("ses_opencode"));
}

#[test]
fn apply_static_env_injects_defaults_but_never_overrides_caller() {
    const STATIC_ENV: &[(&str, &str)] = &[("DISABLE_AUTOUPDATER", "1"), ("FOO", "default")];
    let mut env = std::collections::HashMap::new();
    env.insert("FOO".to_string(), "caller".to_string());
    apply_static_env(&mut env, STATIC_ENV);
    assert_eq!(
        env.get("DISABLE_AUTOUPDATER").map(String::as_str),
        Some("1")
    );
    assert_eq!(
        env.get("FOO").map(String::as_str),
        Some("caller"),
        "a caller-provided value must win over the runtime's static env"
    );
}

// ── spawn diagnostics ────────────────────────────────────────────────────────

/// `E2BIG` is the one spawn failure that is *our* defect, not the machine's: the
/// prompt we built exceeded the OS's per-argument ceiling, so `execve` refused
/// and no agent ever ran. Surfaced raw it reads `Argument list too long (os error
/// 7)` under the `[environment — not an implementation failure]` banner, which is
/// actively misleading — one observed run lost its whole pipeline to it after
/// `s-implement` had already spent 3.8 M tokens.
///
/// Raised by kind rather than by errno 7: the number is E2BIG only where the
/// numbering is POSIX's, and on Windows error 7 is `ERROR_ARENA_TRASHED`, which
/// maps to no kind at all. `spawn_error_message` reads the kind, so the errno
/// would be testing the mapping table instead of the message.
#[test]
fn an_oversized_prompt_is_reported_as_ours_with_the_numbers() {
    let msg = spawn_error_message(
        &std::io::Error::from(std::io::ErrorKind::ArgumentListTooLong),
        "claude",
        "/home/u/.local/bin/claude",
        230_400,
    );

    assert!(
        msg.contains("Nothing about the machine is wrong"),
        "must not read as an environment problem; got: {msg}"
    );
    assert!(
        msg.contains("230400"),
        "the prompt's actual size; got: {msg}"
    );
    assert!(
        msg.contains(&crate::domain::prompt_budget::ARGV_STRING_LIMIT_BYTES.to_string()),
        "the ceiling it cleared; got: {msg}"
    );
    assert!(
        !msg.contains("os error 7"),
        "the raw errno adds nothing a user can act on; got: {msg}"
    );
}

/// The two shapes that are *not* ours must keep saying exactly what they said —
/// a missing binary is the user's to install, and for anything else the OS's own
/// words are better than ours.
#[test]
fn the_other_spawn_failures_read_exactly_as_before() {
    assert_eq!(
        spawn_error_message(
            &std::io::Error::from(std::io::ErrorKind::NotFound),
            "claude",
            "/home/u/.local/bin/claude",
            42,
        ),
        "binary not found at '/home/u/.local/bin/claude'"
    );

    let other = spawn_error_message(
        &std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        "claude",
        "/home/u/.local/bin/claude",
        42,
    );
    assert!(other.starts_with("failed to spawn claude (/home/u/.local/bin/claude): "));
    assert!(
        !other.contains("Nothing about the machine is wrong"),
        "a permission problem really is the machine's; got: {other}"
    );
}
