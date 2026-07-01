use super::*;

#[test]
fn parse_event_text_uses_part_text() {
    let line = r#"{"type":"text","timestamp":1234,"sessionID":"s1","part":{"id":"p1","messageID":"m1","sessionID":"s1","type":"text","text":"hello world","time":{"start":1,"end":2}}}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    match evt {
        AgentEvent::Text { delta } => assert_eq!(delta, "hello world"),
        e => panic!("expected Text, got {:?}", e),
    }
}

#[test]
fn parse_event_text_with_empty_part_text_is_dropped() {
    let line = r#"{"type":"text","part":{"text":""}}"#;
    assert!(parse_opencode_event(line).is_none());
}

#[test]
fn parse_event_step_finish_stop_emits_turn_complete() {
    let line = r#"{"type":"step_finish","timestamp":1234,"sessionID":"s1","part":{"id":"p1","messageID":"m1","sessionID":"s1","reason":"stop","type":"step-finish","tokens":{"total":10447,"input":10408,"output":39,"reasoning":0,"cache":{"write":0,"read":0}},"cost":0}}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    assert!(
        matches!(
            evt,
            AgentEvent::TurnComplete {
                stop_reason: StopReason::EndOfTurn,
                ..
            }
        ),
        "got: {:?}",
        evt
    );
}

#[test]
fn parse_event_step_finish_tool_calls_emits_usage() {
    let line = r#"{"type":"step_finish","part":{"reason":"tool-calls","tokens":{"input":1000,"output":50,"reasoning":0,"cache":{"write":0,"read":0},"total":1050},"cost":0.002}}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    match evt {
        AgentEvent::Usage(u) => {
            assert_eq!(u.input_tokens, 1000);
            assert_eq!(u.output_tokens, 50);
            assert_eq!(u.cost_usd, Some(0.002));
            assert_eq!(u.cache_read_input_tokens, 0);
            assert_eq!(u.cache_creation_input_tokens, 0);
        }
        e => panic!("expected Usage, got {:?}", e),
    }
}

#[test]
fn parse_event_tool_use_formats_as_text() {
    let line = r#"{"type":"tool_use","timestamp":1234,"part":{"type":"tool","tool":"bash","callID":"call_abc","state":{"status":"completed","input":{"command":"ls -la","description":"List dir"},"output":"file1\nfile2","title":"List dir","time":{"start":1,"end":2}}}}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    match evt {
        AgentEvent::Text { delta } => {
            assert!(delta.contains("[tool bash"), "delta was: {}", delta);
            assert!(delta.contains("call_abc"));
            assert!(delta.contains("file1\nfile2"));
        }
        e => panic!("expected Text, got {:?}", e),
    }
}

#[test]
fn parse_event_step_start_is_dropped() {
    let line = r#"{"type":"step_start","part":{"id":"p1","messageID":"m1","sessionID":"s1","type":"step-start"}}"#;
    assert!(parse_opencode_event(line).is_none());
}

#[test]
fn parse_event_tool_use_error_is_dropped() {
    let line = r#"{"type":"tool_use","part":{"tool":"read","callID":"call_err","state":{"status":"error","input":{"filePath":"/nonexistent"},"output":"no such file","title":"Read"}}}"#;
    match parse_opencode_event(line).expect("error tool should produce Text event") {
        AgentEvent::Text { delta } => {
            assert!(
                delta.starts_with("[tool read (error)"),
                "unexpected delta: {delta}"
            );
        }
        e => panic!("expected Text, got {:?}", e),
    }
}

#[test]
fn parse_event_tool_use_running_is_dropped() {
    let line = r#"{"type":"tool_use","part":{"tool":"bash","callID":"call_r","state":{"status":"running","input":{"command":"ls"}}}}"#;
    match parse_opencode_event(line).expect("running tool should produce Text event") {
        AgentEvent::Text { delta } => {
            assert!(
                delta.starts_with("[tool bash (running)"),
                "unexpected delta: {delta}"
            );
        }
        e => panic!("expected Text, got {:?}", e),
    }
}

#[test]
fn parse_event_unknown_part_shape_is_dropped() {
    let line = r#"{"type":"some_new_event","part":{"x":1}}"#;
    assert!(parse_opencode_event(line).is_none());
}

#[test]
fn parse_event_legacy_flat_text_still_works() {
    let line = r#"{"type":"text","delta":"hi"}"#;
    match parse_opencode_event(line).expect("should parse") {
        AgentEvent::Text { delta } => assert_eq!(delta, "hi"),
        e => panic!("expected Text, got {:?}", e),
    }
}

#[test]
fn parse_event_legacy_flat_end_turn_still_works() {
    let line = r#"{"type":"end_turn"}"#;
    assert!(matches!(
        parse_opencode_event(line),
        Some(AgentEvent::TurnComplete { .. })
    ));
}

#[test]
fn parse_event_legacy_nested_session_update_still_works() {
    let line = r#"{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"nested"}}}"#;
    match parse_opencode_event(line).expect("should parse") {
        AgentEvent::Text { delta } => assert_eq!(delta, "nested"),
        e => panic!("expected Text, got {:?}", e),
    }
}

#[test]
fn parse_event_invalid_json_is_dropped() {
    assert!(parse_opencode_event("not json").is_none());
    assert!(parse_opencode_event("").is_none());
}

#[test]
fn parse_event_step_finish_extracts_cache_tokens() {
    // opencode nests cache reads/writes inside tokens.cache.{read,write}.
    let line = r#"{"type":"step_finish","part":{"reason":"tool-calls","tokens":{"input":500,"output":50,"cache":{"read":1000,"write":250},"total":1550},"cost":0.01}}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    match evt {
        AgentEvent::Usage(u) => {
            assert_eq!(u.cache_read_input_tokens, 1000);
            assert_eq!(u.cache_creation_input_tokens, 250);
        }
        e => panic!("expected Usage, got {:?}", e),
    }
}

#[test]
fn parse_event_top_level_usage_update_extracts_cache_tokens() {
    // opencode also emits `usage_update` at the top level.
    let line = r#"{"type":"usage_update","inputTokens":100,"outputTokens":20,"cacheReadInputTokens":500,"cacheCreationInputTokens":100,"costUsd":0.001}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    match evt {
        AgentEvent::Usage(u) => {
            assert_eq!(u.input_tokens, 100);
            assert_eq!(u.output_tokens, 20);
            assert_eq!(u.cache_read_input_tokens, 500);
            assert_eq!(u.cache_creation_input_tokens, 100);
            assert_eq!(u.cost_usd, Some(0.001));
        }
        e => panic!("expected Usage, got {:?}", e),
    }
}

#[test]
fn parse_event_nested_usage_update_extracts_cache_tokens() {
    // And `usage_update` nested under `update.sessionUpdate`.
    let line = r#"{"update":{"sessionUpdate":"usage_update","inputTokens":80,"outputTokens":15,"cacheReadInputTokens":200,"cacheCreationInputTokens":50,"costUsd":0.0005}}"#;
    let evt = parse_opencode_event(line).expect("should parse");
    match evt {
        AgentEvent::Usage(u) => {
            assert_eq!(u.input_tokens, 80);
            assert_eq!(u.output_tokens, 15);
            assert_eq!(u.cache_read_input_tokens, 200);
            assert_eq!(u.cache_creation_input_tokens, 50);
        }
        e => panic!("expected Usage, got {:?}", e),
    }
}

// ── build_opencode_args (token-optimization Tier 1) ──────────────────────

use crate::adapters::agent::opencode::build_opencode_args;
use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_runtime::AgentContext;
use std::collections::HashMap;
use std::sync::Arc;

fn ctx_for_test() -> AgentContext {
    AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "opencode".into(),
        args: vec![],
        env: HashMap::new(),
        cwd: "/tmp/wt".into(),
        model: Some("claude-sonnet-4".into()),
        title: Some("research".into()),
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: PermissionProfile::all_allow(),
        bare_mode: false,
    }
}

#[test]
fn args_no_session_when_captured_id_missing() {
    let args = build_opencode_args(&ctx_for_test(), None);
    assert!(!args.contains(&"--session".to_string()), "got {args:?}");
    assert!(!args.contains(&"--continue".to_string()), "got {args:?}");
}

#[test]
fn args_session_and_continue_emitted_when_captured_id_set() {
    let args = build_opencode_args(&ctx_for_test(), Some("oc-sess-77"));
    let session_idx = args
        .iter()
        .position(|a| a == "--session")
        .expect("--session should be present");
    assert_eq!(args[session_idx + 1], "oc-sess-77");
    let continue_idx = session_idx + 2;
    assert_eq!(args[continue_idx], "--continue");
}

#[test]
fn args_title_passed_through() {
    let args = build_opencode_args(&ctx_for_test(), None);
    let title_idx = args
        .iter()
        .position(|a| a == "--title")
        .expect("--title should be present");
    assert_eq!(args[title_idx + 1], "research");
}
