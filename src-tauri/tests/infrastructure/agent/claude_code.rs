//! Tests for the Claude Code CLI event parser.
//!
//! Fixtures are real JSON lines captured from
//! `claude -p --output-format stream-json --verbose "List the files"`.

use crate::adapters::agent::claude_code::parse_claude_event;
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus};

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
    assert!(matches!(parse_claude_event(line), Some(AgentEvent::ToolCall { .. })));
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
        Some(AgentEvent::ToolCallUpdate { status, .. }) => {
            match status {
                ToolCallStatus::Failed { reason } => {
                    assert!(
                        reason.contains("permissions"),
                        "expected permissions in reason, got {reason:?}"
                    );
                }
                _ => panic!("expected Failed status"),
            }
        }
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }
}

#[test]
fn result_success_end_turn_emits_turn_complete() {
    let line = r#"{"type":"result","subtype":"success","is_error":false,"duration_ms":6116,"duration_api_ms":6781,"num_turns":2,"result":"Here are the files in /tmp","stop_reason":"end_turn","session_id":"bf13ad12","total_cost_usd":0.187}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason }) => {
            assert_eq!(stop_reason, StopReason::EndOfTurn);
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn result_max_tokens_maps_to_max_tokens() {
    let line = r#"{"type":"result","subtype":"success","is_error":false,"stop_reason":"max_tokens","total_cost_usd":0.5}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason }) => {
            assert_eq!(stop_reason, StopReason::MaxTokens);
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
