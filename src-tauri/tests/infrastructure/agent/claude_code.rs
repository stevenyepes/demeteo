//! Tests for the Claude Code CLI event parser.
//!
//! Fixtures are real JSON lines captured from
//! `claude -p --output-format stream-json --verbose "List the files"`.

use crate::adapters::agent::claude_code::parse_claude_event;
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus, Usage};

#[test]
fn system_init_is_dropped() {
    // session_id is captured by drain_lines, not the parser
    let line = r#"{"type":"system","subtype":"init","session_id":"bf13ad12-539e-442b-bed6-09be5b43c82d","model":"MiniMax-M3[1m]"}"#;
    assert!(parse_claude_event(line).is_none());
}

#[test]
fn system_thinking_tokens_is_dropped() {
    let line = r#"{"type":"system","subtype":"thinking_tokens","estimated_tokens":25,"estimated_tokens_delta":25}"#;
    assert!(parse_claude_event(line).is_none());
}

#[test]
fn assistant_text_block_emits_text_event() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Here are the files in /tmp."}]}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::Text { delta }) => {
            assert_eq!(delta, "Here are the files in /tmp.");
        }
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn assistant_thinking_block_is_skipped() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Let me think about this.","signature":"abc"}]}}"#;
    assert!(parse_claude_event(line).is_none());
}

#[test]
fn assistant_tool_use_block_emits_tool_call() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"call_019efb09cb7d71c1a6c5b156","name":"Bash","input":{"command":"ls -la /tmp","description":"List files"}}]}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::ToolCall {
            tool_call_id,
            intercept_id,
            action,
            target,
            preview,
        }) => {
            assert_eq!(tool_call_id, "call_019efb09cb7d71c1a6c5b156");
            assert_eq!(intercept_id, "claude-call_019efb09cb7d71c1a6c5b156");
            assert_eq!(action, ActionKind::RunBash);
            assert_eq!(target, "ls -la /tmp");
            assert!(preview.unwrap_or_default().contains("ls -la /tmp"));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn assistant_write_tool_emits_write_action() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"call_w1","name":"Write","input":{"file_path":"/tmp/hello.txt","content":"world"}}]}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::ToolCall { action, target, .. }) => {
            assert_eq!(action, ActionKind::Write);
            assert_eq!(target, "/tmp/hello.txt");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn assistant_edit_tool_emits_edit_action() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"call_e1","name":"Edit","input":{"file_path":"/tmp/x.rs","old_string":"foo","new_string":"bar"}}]}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::ToolCall { action, target, .. }) => {
            assert_eq!(action, ActionKind::Edit);
            assert_eq!(target, "/tmp/x.rs");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn assistant_read_tool_emits_read_action() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"call_r1","name":"Read","input":{"file_path":"/tmp/x.rs"}}]}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::ToolCall { action, target, .. }) => {
            assert_eq!(action, ActionKind::Read);
            assert_eq!(target, "/tmp/x.rs");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn assistant_tool_use_wins_over_text() {
    // When both text and tool_use are in the same assistant message,
    // the tool call wins (more actionable for the UI / policy layer).
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me run that"},{"type":"tool_use","id":"call_x","name":"Bash","input":{"command":"ls"}}]}}"#;
    assert!(matches!(
        parse_claude_event(line),
        Some(AgentEvent::ToolCall { .. })
    ));
}

#[test]
fn user_tool_result_success_emits_completed_update() {
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_019efb09cb7d71c1a6c5b156","content":"file1\nfile2","is_error":false}]},"tool_use_result":{"stdout":"file1\nfile2","stderr":"","interrupted":false,"isImage":false,"noOutputExpected":false}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::ToolCallUpdate {
            tool_call_id,
            status,
            preview,
        }) => {
            assert_eq!(tool_call_id, "call_019efb09cb7d71c1a6c5b156");
            assert!(matches!(status, ToolCallStatus::Completed));
            assert_eq!(preview.as_deref(), Some("file1\nfile2"));
        }
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }
}

#[test]
fn user_tool_result_error_emits_failed_update_with_reason() {
    // The permission-denied case the user originally hit:
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"call_w1","content":"Claude requested permissions to write to /tmp/hello.txt, but you haven't granted it yet.","is_error":true}]},"tool_use_result":{"stdout":"","stderr":"Claude requested permissions to write to /tmp/hello.txt, but you haven't granted it yet.","interrupted":false,"isImage":false,"noOutputExpected":false}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::ToolCallUpdate { status, .. }) => match status {
            ToolCallStatus::Failed { reason } => {
                assert!(
                    reason.contains("permissions"),
                    "expected permissions in reason, got {reason:?}"
                );
            }
            _ => panic!("expected Failed status"),
        },
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }
}

#[test]
fn result_success_end_turn_emits_turn_complete_with_cost() {
    // After the fix: the `result` event carries total_cost_usd which the
    // parser surfaces on the TurnComplete so the UsageAccumulator can
    // fold it into the turn outcome.
    let line = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":6116,"duration_api_ms":6781,"num_turns":2,"result":"Here are the files in /tmp","stop_reason":"end_turn","session_id":"bf13ad12","total_cost_usd":0.187}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason, usage }) => {
            assert_eq!(stop_reason, StopReason::EndOfTurn);
            // No usage block in this fixture → cost_usd is the only data.
            let u = usage.expect("expected usage snapshot on result event");
            assert_eq!(u.input_tokens, 0);
            assert_eq!(u.output_tokens, 0);
            assert!((u.cost_usd.expect("cost present") - 0.187).abs() < 1e-9);
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn result_max_tokens_maps_to_max_tokens_and_carries_cost() {
    let line = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"max_tokens","total_cost_usd":0.5}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason, usage }) => {
            assert_eq!(stop_reason, StopReason::MaxTokens);
            let u = usage.expect("expected usage snapshot on result event");
            assert!((u.cost_usd.expect("cost present") - 0.5).abs() < 1e-9);
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn result_with_full_usage_block_emits_usage_snapshot() {
    // Anthropic SDK cost-tracking confirms the `result` event carries
    // the full usage block: input/output tokens plus cache creation /
    // read tokens. All four numeric fields must surface.
    let line = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","total_cost_usd":0.187,"usage":{"input_tokens":100,"output_tokens":50,"cache_creation_input_tokens":500,"cache_read_input_tokens":1000}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason, usage }) => {
            assert_eq!(stop_reason, StopReason::EndOfTurn);
            assert_eq!(
                usage,
                Some(Usage {
                    input_tokens: 100,
                    output_tokens: 50,
                    cost_usd: Some(0.187),
                    cache_read_input_tokens: 1000,
                    cache_creation_input_tokens: 500,
                })
            );
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn result_missing_usage_block_emits_turn_complete_with_none_usage() {
    // Tool-only turns (no API call) can have a result event with neither
    // total_cost_usd nor usage block — usage must be None, not panic.
    let line = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"end_turn","session_id":"abc"}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason, usage }) => {
            assert_eq!(stop_reason, StopReason::EndOfTurn);
            assert!(usage.is_none());
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn result_error_emits_error_event() {
    let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"claude API error","stop_reason":"error"}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::Error { code, message, .. }) => {
            assert_eq!(code, "cli_error");
            assert_eq!(message, "claude API error");
        }
        other => panic!("expected Error, got {other:?}"),
    }
}

#[test]
fn unknown_type_is_dropped() {
    let line = r#"{"type":"stream_event","event":{"type":"message_start"}}"#;
    assert!(parse_claude_event(line).is_none());
}

#[test]
fn malformed_json_is_dropped() {
    assert!(parse_claude_event("not json").is_none());
    assert!(parse_claude_event("").is_none());
}

// ── build_claude_args (token-optimization Tier 1) ─────────────────────────

use crate::adapters::agent::claude_code::build_claude_args;
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_runtime::AgentContext;
use std::collections::HashMap;
use std::sync::Arc;

#[path = "_arg_test_stubs.rs"]
mod stubs;
use stubs::{StubAgentExec, StubExec};

fn ctx_for_test(bare_mode: bool) -> AgentContext {
    AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "claude".into(),
        args: vec![],
        env: HashMap::new(),
        cwd: ".".into(),
        model: Some("claude-sonnet-4".into()),
        title: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: PermissionProfile::all_allow(),
        bare_mode,
    }
}

#[test]
fn args_no_resume_when_session_id_missing() {
    let args = build_claude_args(&ctx_for_test(false), None);
    assert!(!args.contains(&"--resume".to_string()), "got {args:?}");
}

#[test]
fn args_resume_emitted_when_captured_session_id_set() {
    let args = build_claude_args(&ctx_for_test(false), Some("sess-abc-123"));
    let resume_idx = args
        .iter()
        .position(|a| a == "--resume")
        .expect("--resume should be present");
    assert_eq!(args[resume_idx + 1], "sess-abc-123");
}

#[test]
fn isolation_flags_only_when_bare_mode_true() {
    // Isolated pipeline mode emits the cache-stability flags but NOT
    // `--bare` — `--bare` sets CLAUDE_CODE_SIMPLE=1 and disables
    // keychain/OAuth reads, which we rely on so Claude authenticates
    // (and refreshes) its own credential. See `build_claude_args`.
    let with_bare = build_claude_args(&ctx_for_test(true), None);
    assert!(
        !with_bare.contains(&"--bare".to_string()),
        "--bare must NOT be emitted (it disables keychain auth): got {with_bare:?}"
    );
    assert!(with_bare.contains(&"--exclude-dynamic-system-prompt-sections".to_string()));
    assert!(with_bare.contains(&"--strict-mcp-config".to_string()));
    let src_idx = with_bare
        .iter()
        .position(|a| a == "--setting-sources")
        .expect("--setting-sources should be present in bare mode");
    // user + project (so the user's committed project skills/CLAUDE.md
    // load) but not machine-local `settings.local.json`.
    assert_eq!(with_bare[src_idx + 1], "user,project");

    let without_bare = build_claude_args(&ctx_for_test(false), None);
    assert!(!without_bare.contains(&"--exclude-dynamic-system-prompt-sections".to_string()));
    assert!(!without_bare.contains(&"--setting-sources".to_string()));
    assert!(!without_bare.contains(&"--strict-mcp-config".to_string()));
}

#[test]
fn args_model_passed_through() {
    let args = build_claude_args(&ctx_for_test(false), None);
    let model_idx = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model should be present");
    assert_eq!(args[model_idx + 1], "claude-sonnet-4");
}

#[test]
fn args_print_and_dangerously_skip_always_present() {
    let args = build_claude_args(&ctx_for_test(true), Some("sess-1"));
    assert!(args.contains(&"--print".to_string()));
    assert!(args.contains(&"--dangerously-skip-permissions".to_string()));
    assert!(args.contains(&"--output-format".to_string()));
    assert!(args.contains(&"stream-json".to_string()));
}

#[test]
fn args_never_emit_settings() {
    // We used to pass `--settings <path>` to wire up an `apiKeyHelper`.
    // That path is broken: Claude invokes the helper via `/bin/sh -c`
    // which splits on whitespace, so any path containing a space (the
    // macOS app data dir under `~/Library/Application Support/`) fails
    // with exit 127. We dropped `--settings` (and `--bare`) entirely:
    // Claude reads and refreshes its own keychain/OAuth credential
    // natively, so Demeteo injects no auth at all. (Note: `--settings`
    // is distinct from the `--setting-sources` flag emitted in bare
    // mode; this asserts the former is absent.)
    let with_bare = build_claude_args(&ctx_for_test(true), Some("sess-1"));
    assert!(
        !with_bare.contains(&"--settings".to_string()),
        "--settings must NOT be emitted: got {with_bare:?}"
    );
    let without_bare = build_claude_args(&ctx_for_test(false), None);
    assert!(
        !without_bare.contains(&"--settings".to_string()),
        "--settings must NOT be emitted: got {without_bare:?}"
    );
}
