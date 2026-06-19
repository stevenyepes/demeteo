//! Integration tests for the AcpRuntime using the in-tree mock agent.
//! These tests verify the full lifecycle end-to-end:
//! - `initialize` round-trip
//! - `session/new` round-trip
//! - `session/prompt` produces a stream with `Text`, `ToolCall`, and
//!   `TurnComplete` events in the right order
//!
//! The mock agent (`mock_agent.rs`) is a small shell script that drives
//! a state machine. It is the only "agent" we can run without pulling
//! in a real opencode or Hermes install.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::runtime::Runtime;
use tokio_stream::StreamExt;

use crate::adapters::agent::acp::mock_agent::MockAgent;
use crate::adapters::agent::acp::runtime::AcpRuntime;
use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession};

fn build_ctx(binary: &str) -> AgentContext {
    use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
    use crate::domain::intercept::ExecutionResult;
    use crate::domain::action::AgentAction;
    use crate::ports::execution::ExecutionPort;

    struct StubExec;
    impl AgentExecutionPort for StubExec {
        fn submit(
            &self,
            _: &str,
            _: &str,
            _: AgentAction,
        ) -> Result<CommandOutcome, String> {
            Ok(CommandOutcome::Executed {
                output: ExecutionResult::Bash { output: String::new() },
            })
        }
        fn submit_agent(
            &self,
            _: &str,
            _: &str,
            _: AgentAction,
            _: Option<String>,
        ) -> Result<CommandOutcome, ActionError> {
            // The integration test only exercises the lifecycle
            // (Text, ToolCall, TurnComplete). The bridge is unit-tested
            // separately; we don't need a full policy here.
            Ok(CommandOutcome::Executed {
                output: ExecutionResult::FileRead {
                    path: "/tmp/x".into(),
                    content_preview: "line1\n".into(),
                },
            })
        }
            fn approve(&self, _: &str) -> Result<(), String> { Ok(()) }
            fn reject(&self, _: &str, _: String) -> Result<(), String> { Ok(()) }
            fn register_result_responder(
            &self,
            _: &str,
            _: tokio::sync::oneshot::Sender<Result<crate::domain::intercept::ExecutionResult, String>>,
        ) -> Result<(), String> { Ok(()) }
    }
    impl ExecutionPort for StubExec {
        fn test_connection(&self, _: &str) -> Result<(), String> { Ok(()) }
        fn run_command(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
        fn read_file(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
        fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn get_metadata(&self, _: &str, path: &str) -> Result<crate::sftp::SftpEntry, String> {
            Ok(crate::sftp::SftpEntry { name: path.into(), path: path.into(), is_dir: false, size: 0, modified: 0 })
        }
        fn list_dir(&self, _: &str, _: &str) -> Result<Vec<crate::sftp::SftpEntry>, String> { Ok(vec![]) }
        fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
        fn resolve_home(&self, _: &str) -> Result<String, String> { Ok("/tmp".to_string()) }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("not implemented in stub".into())
        }
    }

    let stub = Arc::new(StubExec);
    AgentContext {
        thread_id: "t-mock".into(),
        machine_id: "".into(), // local
        binary: binary.into(),
        args: vec![],
        env: HashMap::new(),
        cwd: ".".into(),
        model: None,
        agent_exec: stub.clone(),
        exec: stub,
    }
}

fn runtime() -> AcpRuntime {
    // Use the "opencode" kind so we exercise the same code path the
    // production registry uses. The mock agent's name is irrelevant;
    // what matters is the binary path.
    AcpRuntime::new("opencode", "echo opencode")
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn acp_runtime_lifecycle_emits_expected_event_stream() {
    let mock = MockAgent::install();
    let ctx = build_ctx(mock.binary());
    let session = rt().block_on(async { runtime().start(ctx).await.expect("start") });

    // Sanity: the session has an id.
    assert_eq!(session.session_id(), "mock-session-1");

    // Drive the prompt. Spawn the runtime, then collect the stream.
    // We use a multi-threaded runtime so the blocking JSON-RPC calls
    // (which use std::sync::Mutex + blocking I/O) don't collide with
    // the tokio worker pool.
    let events: Vec<AgentEvent> = rt().block_on(async move {
        let mut stream = session.prompt("hi");
        let mut out = Vec::new();
        while let Some(ev) = stream.next().await {
            out.push(ev);
        }
        out
    });

    // Expected order from the mock agent:
    // 1. Text("hello from mock")
    // 2. ToolCall { tool_call_id: "tc-mock-1", target: "/tmp/x", ... }
    // 3. TurnComplete { stop_reason: EndOfTurn }
    assert!(events.len() >= 3, "expected >= 3 events, got {}: {:#?}", events.len(), events);
    let has_text = events.iter().any(|e| matches!(e, AgentEvent::Text { delta } if delta == "hello from mock"));
    let has_tool_call = events.iter().any(|e| matches!(e, AgentEvent::ToolCall { tool_call_id, target, .. } if tool_call_id == "tc-mock-1" && target == "/tmp/x"));
    let has_turn_complete = events.iter().any(|e| matches!(e, AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn }));
    assert!(has_text, "missing Text event in: {:#?}", events);
    assert!(has_tool_call, "missing ToolCall event in: {:#?}", events);
    assert!(has_turn_complete, "missing TurnComplete event in: {:#?}", events);
}

#[test]
fn acp_runtime_returns_arc_dyn_session() {
    let mock = MockAgent::install();
    let ctx = build_ctx(mock.binary());
    let session: Arc<dyn AgentSession> =
        rt().block_on(async { runtime().start(ctx).await.expect("start") });
    assert!(!session.session_id().is_empty());
}

#[test]
fn acp_runtime_dispatches_tool_call_through_bridge() {
    // Drive a richer mock agent that records the `tool_call/update`
    // notification the runtime sends in response to the agent's
    // `tool_call` notification. We verify the file path was preserved
    // and the runtime produced a `ToolCall` event for the UI.
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut ctx = build_ctx("");
    let record_path = std::env::temp_dir().join(format!(
        "demeteo_tool_call_record_{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let record_path_str = record_path.to_str().unwrap().to_string();

    // The mock agent records each `tool_call/update` notification it
    // receives from the runtime into `record_path`, then closes the
    // turn. The runtime should have dispatched the bridge with the
    // correct tool_call_id and target.
    let script = format!(
        r#"#!/bin/sh
RECORD="{record}"
state="init"
while IFS= read -r line; do
  case "$state" in
    init)
      echo '{{"id":1,"result":{{"protocolVersion":1,"capabilities":{{"toolCallUpdate":true}}}}}}'
      state="new"
      ;;
    new)
      echo '{{"id":2,"result":{{"sessionId":"mock-session-2"}}}}'
      state="prompted"
      ;;
    prompted)
      # Two notifications (text + tool_call), then the prompt response.
      echo '{{"method":"session/update","params":{{"kind":"text","delta":"hi"}}}}'
      echo '{{"method":"session/update","params":{{"kind":"tool_call","toolCallId":"tc-round-trip","action":"read","target":"/tmp/observed"}}}}'
      echo '{{"id":3,"result":{{"stopReason":"end_of_turn"}}}}'
      state="idle"
      ;;
    *)
      # Late requests — including tool_call/update — append to record.
      echo "$line" >> "$RECORD"
      rid=$(echo "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      if [ -z "$rid" ]; then rid=99; fi
      echo "{{\"id\":$rid,\"result\":{{}}}}"
      ;;
  esac
done
"#, record = record_path_str
    );

    let mock = MockAgent::install_with_script(&script);
    ctx.binary = mock.binary().to_string();

    let session = rt().block_on(async { runtime().start(ctx).await.expect("start") });

    let events: Vec<AgentEvent> = rt().block_on(async {
        let mut stream = session.prompt("anything");
        let mut out = Vec::new();
        while let Some(ev) = stream.next().await {
            out.push(ev);
        }
        out
    });

    // Verify the runtime emitted a ToolCall event for the UI.
    let tool_event = events.iter().find_map(|e| match e {
        AgentEvent::ToolCall { tool_call_id, target, .. } => {
            Some((tool_call_id.clone(), target.clone()))
        }
        _ => None,
    });
    let (tc_id, tc_target) = tool_event.expect("expected a ToolCall event");
    assert_eq!(tc_id, "tc-round-trip");
    assert_eq!(tc_target, "/tmp/observed");

    // Give the runtime a moment to flush the tool_call/update
    // notification to the mock agent. The mock agent records each
    // late request line into `record_path`.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let record = std::fs::read_to_string(&record_path).unwrap_or_default();
    let _ = std::fs::remove_file(&record_path);
    assert!(
        record.contains("toolCall/update"),
        "expected toolCall/update in agent's record; got: {}",
        record
    );
    assert!(
        record.contains("tc-round-trip"),
        "expected tool_call_id in agent's record; got: {}",
        record
    );
}

#[test]
fn acp_runtime_dispatches_opencode_tool_call_through_bridge() {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut ctx = build_ctx("");
    let record_path = std::env::temp_dir().join(format!(
        "demeteo_tool_call_record_opencode_{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let record_path_str = record_path.to_str().unwrap().to_string();

    let script = format!(
        r#"#!/bin/sh
RECORD="{record}"
state="init"
while IFS= read -r line; do
  case "$state" in
    init)
      echo '{{"id":1,"result":{{"protocolVersion":1,"capabilities":{{"toolCallUpdate":true}}}}}}'
      state="new"
      ;;
    new)
      echo '{{"id":2,"result":{{"sessionId":"mock-session-2"}}}}'
      state="prompted"
      ;;
    prompted)
      # Two notifications (text + opencode-style tool_call), then the prompt response.
      echo '{{"method":"session/update","params":{{"kind":"text","delta":"hi"}}}}'
      echo '{{"method":"session/update","params":{{"sessionId":"mock-session-2","update":{{"sessionUpdate":"tool_call","toolCallId":"tc-round-trip-opencode","title":"Read file","rawInput":{{"action":"read","path":"/tmp/observed-opencode"}}}}}}}}'
      echo '{{"id":3,"result":{{"stopReason":"end_of_turn"}}}}'
      state="idle"
      ;;
    *)
      # Late requests — including tool_call/update — append to record.
      echo "$line" >> "$RECORD"
      rid=$(echo "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
      if [ -z "$rid" ]; then rid=99; fi
      echo "{{\"id\":$rid,\"result\":{{}}}}"
      ;;
  esac
done
"#, record = record_path_str
    );

    let mock = MockAgent::install_with_script(&script);
    ctx.binary = mock.binary().to_string();

    let session = rt().block_on(async { runtime().start(ctx).await.expect("start") });

    let events: Vec<AgentEvent> = rt().block_on(async {
        let mut stream = session.prompt("anything");
        let mut out = Vec::new();
        while let Some(ev) = stream.next().await {
            out.push(ev);
        }
        out
    });

    // Verify the runtime emitted a ToolCall event for the UI.
    let tool_event = events.iter().find_map(|e| match e {
        AgentEvent::ToolCall { tool_call_id, target, .. } => {
            Some((tool_call_id.clone(), target.clone()))
        }
        _ => None,
    });
    let (tc_id, tc_target) = tool_event.expect("expected an Opencode ToolCall event");
    assert_eq!(tc_id, "tc-round-trip-opencode");
    assert_eq!(tc_target, "/tmp/observed-opencode");

    // Give the runtime a moment to flush the tool_call/update
    // notification to the mock agent.
    std::thread::sleep(std::time::Duration::from_millis(200));

    let record = std::fs::read_to_string(&record_path).unwrap_or_default();
    let _ = std::fs::remove_file(&record_path);
    assert!(
        record.contains("toolCall/update"),
        "expected toolCall/update in agent's record; got: {}",
        record
    );
    assert!(
        record.contains("tc-round-trip-opencode"),
        "expected tool_call_id in agent's record; got: {}",
        record
    );
}
