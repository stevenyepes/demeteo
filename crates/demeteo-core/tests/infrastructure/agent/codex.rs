//! Tests for the Codex CLI event parser + arg builder.
//!
//! Fixtures are real JSON lines captured from
//! `codex exec "<prompt>" --json --sandbox workspace-write` (codex-cli
//! 0.142.3). The golden transcript under
//! `tests/fixtures/agent_transcripts/codex/0.142.3/` is the seed fixture for
//! Epic A3's conformance harness — a replay test here turns an upstream
//! wire-format change into a red CI check instead of a silent parse failure.

use crate::adapters::agent::codex::{
    build_codex_args, codex_output_schema_args, parse_codex_event,
};
use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus};
use crate::domain::permission::{PermissionProfile, StepCapability};
use crate::ports::agent_runtime::AgentContext;
use std::collections::HashMap;
use std::sync::Arc;

// ── Event parser ───────────────────────────────────────────────────────

#[test]
fn thread_started_is_dropped() {
    // thread_id is captured by drain_lines, not the parser.
    let line = r#"{"type":"thread.started","thread_id":"019f4ce2-2d91-7442-be57-b599da8e827b"}"#;
    assert!(parse_codex_event(line).is_none());
}

#[test]
fn turn_started_is_dropped() {
    assert!(parse_codex_event(r#"{"type":"turn.started"}"#).is_none());
}

#[test]
fn agent_message_emits_text() {
    let line = r#"{"type":"item.completed","item":{"id":"item_1","type":"agent_message","text":"I'll create the file."}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::Text { delta }) => assert_eq!(delta, "I'll create the file."),
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn reasoning_item_is_skipped() {
    let line = r#"{"type":"item.completed","item":{"id":"item_9","type":"reasoning","text":"Let me think."}}"#;
    assert!(parse_codex_event(line).is_none());
}

#[test]
fn command_started_emits_tool_call() {
    let line = r#"{"type":"item.started","item":{"id":"item_2","type":"command_execution","command":"ls -la","aggregated_output":"","exit_code":null,"status":"in_progress"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::ToolCall {
            tool_call_id,
            intercept_id,
            action,
            target,
            ..
        }) => {
            assert_eq!(tool_call_id, "item_2");
            assert_eq!(intercept_id, "codex-item_2");
            assert_eq!(action, ActionKind::RunBash);
            assert_eq!(target, "ls -la");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn command_completed_ok_emits_completed_update() {
    let line = r#"{"type":"item.completed","item":{"id":"item_2","type":"command_execution","command":"ls","aggregated_output":"a\nb\n","exit_code":0,"status":"completed"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::ToolCallUpdate {
            tool_call_id,
            status,
            preview,
        }) => {
            assert_eq!(tool_call_id, "item_2");
            assert!(matches!(status, ToolCallStatus::Completed));
            assert_eq!(preview.as_deref(), Some("a\nb\n"));
        }
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }
}

#[test]
fn command_completed_nonzero_exit_emits_failed_update() {
    let line = r#"{"type":"item.completed","item":{"id":"item_4","type":"command_execution","command":"boom","aggregated_output":"Error: nope\n","exit_code":2,"status":"failed"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::ToolCallUpdate { status, .. }) => match status {
            ToolCallStatus::Failed { reason } => assert!(reason.contains("nope")),
            other => panic!("expected Failed, got {other:?}"),
        },
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }
}

#[test]
fn file_change_emits_edit_tool_call() {
    let line = r#"{"type":"item.completed","item":{"id":"item_5","type":"file_change","changes":[{"path":"src/lib.rs","kind":"update"}],"status":"completed"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::ToolCall { action, target, .. }) => {
            assert_eq!(action, ActionKind::Edit);
            assert_eq!(target, "src/lib.rs");
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn file_change_add_emits_write_tool_call() {
    let line = r#"{"type":"item.completed","item":{"id":"item_5","type":"file_change","changes":[{"path":"new.txt","kind":"add"}],"status":"completed"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::ToolCall { action, .. }) => assert_eq!(action, ActionKind::Write),
        other => panic!("expected ToolCall, got {other:?}"),
    }
}

#[test]
fn generic_item_error_is_recoverable() {
    // A genuine per-item error is surfaced but must NOT abort the turn.
    let line = r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"tool call failed"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::Error { recoverable, .. }) => assert!(recoverable),
        other => panic!("expected recoverable Error, got {other:?}"),
    }
}

#[test]
fn model_metadata_fallback_notice_is_dropped() {
    // codex's "Model metadata for `<slug>` not found. Defaulting to fallback
    // metadata..." is routine for any custom provider/model and is not an
    // error — it must be dropped, not surfaced as an AgentEvent::Error.
    let line = r#"{"type":"item.completed","item":{"id":"item_0","type":"error","message":"Model metadata for `minimax-coding-plan/MiniMax-M3` not found. Defaulting to fallback metadata; this can degrade performance and cause issues."}}"#;
    assert!(parse_codex_event(line).is_none());
}

#[test]
fn turn_failed_is_terminal_error() {
    let line = r#"{"type":"turn.failed","error":{"message":"context window exceeded"}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::Error {
            recoverable,
            message,
            ..
        }) => {
            assert!(!recoverable);
            assert!(message.contains("context window"));
        }
        other => panic!("expected terminal Error, got {other:?}"),
    }
}

#[test]
fn turn_completed_emits_turn_complete_with_usage() {
    let line = r#"{"type":"turn.completed","usage":{"input_tokens":22039,"cached_input_tokens":11008,"output_tokens":75,"reasoning_output_tokens":0}}"#;
    match parse_codex_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason, usage }) => {
            assert_eq!(stop_reason, StopReason::EndOfTurn);
            let u = usage.expect("usage present");
            assert_eq!(u.input_tokens, 22039);
            assert_eq!(u.output_tokens, 75);
            assert_eq!(u.cache_read_input_tokens, 11008);
            assert!(u.cost_usd.is_none());
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn unknown_top_level_type_is_dropped() {
    assert!(parse_codex_event(r#"{"type":"future.event","x":1}"#).is_none());
    assert!(parse_codex_event("not json").is_none());
}

// ── Golden-transcript replay (seeds Epic A3.1) ─────────────────────────

#[test]
fn golden_transcript_replays_to_expected_event_sequence() {
    let raw = include_str!("../../fixtures/agent_transcripts/codex/0.142.3/first-run.jsonl");
    let events: Vec<AgentEvent> = raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_codex_event)
        .collect();

    // thread.started + turn.started drop, and so does the item_0
    // model-metadata fallback notice (benign, custom-model provider). Every
    // remaining line yields one event:
    // text → tool_call → tool_update → text → turn_complete.
    assert_eq!(events.len(), 5, "got {events:?}");
    assert!(matches!(events[0], AgentEvent::Text { .. }));
    assert!(matches!(events[1], AgentEvent::ToolCall { .. }));
    assert!(matches!(
        events[2],
        AgentEvent::ToolCallUpdate {
            status: ToolCallStatus::Completed,
            ..
        }
    ));
    assert!(matches!(events[3], AgentEvent::Text { .. }));
    assert!(matches!(
        events[4],
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: Some(_)
        }
    ));
}

// ── Arg builder ────────────────────────────────────────────────────────

fn ctx_with(model: Option<&str>, perms: PermissionProfile) -> AgentContext {
    AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "codex".into(),
        args: vec![],
        env: HashMap::new(),
        cwd: ".".into(),
        model: model.map(|s| s.to_string()),
        effort: None,
        title: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: perms,
        bare_mode: false,
    }
}

fn arg_pair<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .map(|i| args[i + 1].as_str())
}

#[test]
fn args_are_exec_json_by_default() {
    let args = build_codex_args(&ctx_with(None, PermissionProfile::all_allow()), None, "hi");
    assert_eq!(args.first().map(String::as_str), Some("exec"));
    assert!(args.contains(&"--json".to_string()));
    assert!(!args.contains(&"resume".to_string()));
    // Prompt is the trailing positional.
    assert_eq!(args.last().map(String::as_str), Some("hi"));
}

#[test]
fn args_resume_replays_captured_thread() {
    let args = build_codex_args(
        &ctx_with(None, PermissionProfile::all_allow()),
        Some("019f4ce2-2d91-7442"),
        "next",
    );
    assert_eq!(args[0], "exec");
    assert_eq!(args[1], "resume");
    assert_eq!(args[2], "019f4ce2-2d91-7442");
    assert!(args.contains(&"--json".to_string()));
}

#[test]
fn args_write_allowed_selects_workspace_write() {
    let args = build_codex_args(&ctx_with(None, PermissionProfile::all_allow()), None, "");
    assert!(
        args.contains(&"sandbox_mode=workspace-write".to_string()),
        "got {args:?}"
    );
    // network allowed → escalation flag present.
    assert!(args.contains(&"sandbox_workspace_write.network_access=true".to_string()));
}

#[test]
fn args_read_only_capability_selects_read_only_sandbox() {
    let perms = StepCapability::ReadOnly.base_profile();
    let args = build_codex_args(&ctx_with(None, perms), None, "");
    assert!(
        args.contains(&"sandbox_mode=read-only".to_string()),
        "got {args:?}"
    );
    // read-only → no network escalation.
    assert!(!args.iter().any(|a| a.contains("network_access")));
}

#[test]
fn args_never_block_on_approval() {
    let args = build_codex_args(&ctx_with(None, PermissionProfile::all_allow()), None, "");
    assert!(args.contains(&"approval_policy=never".to_string()));
}

#[test]
fn args_model_passed_through() {
    let args = build_codex_args(
        &ctx_with(Some("gpt-5.5-codex"), PermissionProfile::all_allow()),
        None,
        "",
    );
    assert_eq!(arg_pair(&args, "--model"), Some("gpt-5.5-codex"));
}

#[test]
fn output_schema_args_are_reusable() {
    let frag = codex_output_schema_args("schema.json", "out.json");
    assert_eq!(
        frag,
        vec!["--output-schema", "schema.json", "-o", "out.json"]
            .into_iter()
            .map(String::from)
            .collect::<Vec<_>>()
    );
}

// ── Effort ───────────────────────────────────────────────────────────────

use crate::domain::models::EffortLevel;

fn ctx_with_effort(effort: Option<EffortLevel>) -> AgentContext {
    AgentContext {
        effort,
        ..ctx_with(None, PermissionProfile::all_allow())
    }
}

/// The value of the `-c <key>=<value>` pair for `key`, asserting the `-c`
/// immediately precedes it. Codex emits several `-c` pairs (sandbox_mode,
/// approval_policy, …), so "an `-c` exists somewhere" proves nothing.
fn config_pair<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
    let prefix = format!("{key}=");
    let i = args.iter().position(|a| a.starts_with(&prefix))?;
    assert_eq!(
        args.get(i - 1).map(String::as_str),
        Some("-c"),
        "`{key}` must be preceded by its own -c: got {args:?}"
    );
    args[i].strip_prefix(&prefix)
}

#[test]
fn args_effort_emitted_as_adjacent_config_pair() {
    let args = build_codex_args(&ctx_with_effort(Some(EffortLevel::High)), None, "");
    assert_eq!(config_pair(&args, "model_reasoning_effort"), Some("high"));
}

#[test]
fn args_effort_clamped_to_what_codex_supports() {
    // Codex has no `max`, and it does not validate — an unknown value is
    // wrapped as Custom(String) and sent. So the clamp is ours to do.
    let args = build_codex_args(&ctx_with_effort(Some(EffortLevel::Max)), None, "");
    assert_eq!(config_pair(&args, "model_reasoning_effort"), Some("xhigh"));
}

#[test]
fn args_no_effort_when_unset() {
    let args = build_codex_args(&ctx_with_effort(None), None, "");
    assert!(
        !args.iter().any(|a| a.starts_with("model_reasoning_effort")),
        "got {args:?}"
    );
}
