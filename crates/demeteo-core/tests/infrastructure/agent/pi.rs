//! Tests for the pi event parser, arg builder, and model-table parser.
//!
//! The fixtures under `tests/fixtures/agent_transcripts/pi/0.83.0/` are raw
//! stdout captured from real `pi --mode json` runs. Replaying them here is what
//! turns an upstream wire-format change into a red CI check instead of a silent
//! parse failure — see `docs/adapters/CONTRIBUTING-AN-AGENT.md` step 5.

use crate::adapters::agent::cli_runtime::session_id_from_line;
use crate::adapters::agent::pi::{
    build_pi_args, excluded_tools_for, parse_pi_event, parse_pi_model_table,
};
use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
use crate::domain::action::ActionKind;
use crate::domain::agent_event::{AgentEvent, StopReason, ToolCallStatus};
use crate::domain::models::{AgentKind, EffortLevel};
use crate::domain::permission::{Access, PermissionProfile, StepCapability};
use crate::domain::usage::UsageAccumulator;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::pricing::{ModelPrice, PricingTable};
use std::collections::HashMap;
use std::sync::Arc;

const TEXT_ONLY: &str = include_str!("../../fixtures/agent_transcripts/pi/0.83.0/text-only.jsonl");
const MULTI_TURN: &str =
    include_str!("../../fixtures/agent_transcripts/pi/0.83.0/tool-call-multi-turn.jsonl");

fn replay(raw: &str) -> Vec<AgentEvent> {
    raw.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(parse_pi_event)
        .collect()
}

// ── Event parser ───────────────────────────────────────────────────────

#[test]
fn session_header_is_dropped_by_the_parser() {
    let line =
        r#"{"type":"session","version":3,"id":"019fbb89-d498-7012-8fd8-fcdd6b505378","cwd":"/w"}"#;
    assert!(parse_pi_event(line).is_none());
}

#[test]
fn only_text_delta_message_updates_yield_prose() {
    let delta = r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":"pong"}}"#;
    match parse_pi_event(delta) {
        Some(AgentEvent::Text { delta }) => assert_eq!(delta, "pong"),
        other => panic!("expected Text, got {other:?}"),
    }

    // `text_end` repeats the whole accumulated `content` and the `toolcall_*`
    // family re-streams args that `tool_execution_start` already carries
    // parsed. Either one widening this arm double-counts the stream.
    for other in [
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_start","contentIndex":0}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_end","contentIndex":0,"content":"pong"}}"#,
        r#"{"type":"message_update","assistantMessageEvent":{"type":"toolcall_delta","delta":"{\"path\":\".\"}"}}"#,
    ] {
        assert!(parse_pi_event(other).is_none(), "leaked: {other}");
    }
}

#[test]
fn empty_text_delta_is_dropped() {
    let line =
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":""}}"#;
    assert!(parse_pi_event(line).is_none());
}

#[test]
fn tool_names_map_to_the_action_the_policy_layer_gates() {
    let cases = [
        (r#"{"path":"."}"#, "ls", ActionKind::Read, "."),
        (r#"{"path":"a.rs"}"#, "read", ActionKind::Read, "a.rs"),
        (
            r#"{"pattern":"fn main"}"#,
            "grep",
            ActionKind::Read,
            "fn main",
        ),
        (r#"{"pattern":"*.rs"}"#, "find", ActionKind::Read, "*.rs"),
        (r#"{"path":"a.rs"}"#, "edit", ActionKind::Edit, "a.rs"),
        (
            r#"{"path":"new.txt"}"#,
            "write",
            ActionKind::Write,
            "new.txt",
        ),
        (
            r#"{"command":"cargo test"}"#,
            "bash",
            ActionKind::RunBash,
            "cargo test",
        ),
        // Anything gentler for an unrecognised tool routes it past the policy
        // layer as a read.
        (
            r#"{"command":"whatever"}"#,
            "some_extension",
            ActionKind::RunBash,
            "whatever",
        ),
    ];
    for (args, tool, want_action, want_target) in cases {
        let line = format!(
            r#"{{"type":"tool_execution_start","toolCallId":"c1","toolName":"{tool}","args":{args}}}"#
        );
        match parse_pi_event(&line) {
            Some(AgentEvent::ToolCall {
                tool_call_id,
                intercept_id,
                action,
                target,
                ..
            }) => {
                assert_eq!(tool_call_id, "c1");
                assert_eq!(intercept_id, "pi-c1");
                assert_eq!(action, want_action, "tool {tool}");
                assert_eq!(target, want_target, "tool {tool}");
            }
            other => panic!("expected ToolCall for {tool}, got {other:?}"),
        }
    }
}

#[test]
fn tool_execution_end_reports_success_and_failure() {
    let ok = r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"ls","result":{"content":[{"type":"text","text":"a.txt"}]},"isError":false}"#;
    match parse_pi_event(ok) {
        Some(AgentEvent::ToolCallUpdate {
            tool_call_id,
            status,
            preview,
        }) => {
            assert_eq!(tool_call_id, "c1");
            assert!(matches!(status, ToolCallStatus::Completed));
            assert_eq!(preview.as_deref(), Some("a.txt"));
        }
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }

    let bad = r#"{"type":"tool_execution_end","toolCallId":"c2","toolName":"bash","result":{"content":[{"type":"text","text":"exit 2: nope"}]},"isError":true}"#;
    match parse_pi_event(bad) {
        Some(AgentEvent::ToolCallUpdate { status, .. }) => match status {
            ToolCallStatus::Failed { reason } => assert!(reason.contains("nope")),
            other => panic!("expected Failed, got {other:?}"),
        },
        other => panic!("expected ToolCallUpdate, got {other:?}"),
    }
}

/// The whole point of the G1 change: a `turn_end` counts **one model request**,
/// so it must never reach the accumulator as the maxed `Usage` snapshot shape.
#[test]
fn turn_end_is_a_usage_delta_never_a_snapshot() {
    let line = r#"{"type":"turn_end","message":{"stopReason":"toolUse","usage":{"input":1058,"output":18,"cacheRead":7,"cacheWrite":3,"totalTokens":1076,"cost":{"total":0.001166}}}}"#;
    match parse_pi_event(line) {
        Some(AgentEvent::UsageDelta(u)) => {
            assert_eq!(u.input_tokens, 1058);
            assert_eq!(u.output_tokens, 18);
            assert_eq!(u.cache_read_input_tokens, 7);
            assert_eq!(u.cache_creation_input_tokens, 3);
            assert_eq!(u.cost_usd, Some(0.001166));
        }
        other => panic!("expected UsageDelta, got {other:?}"),
    }
}

#[test]
fn agent_end_is_terminal_and_carries_no_usage() {
    let line = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#;
    match parse_pi_event(line) {
        Some(AgentEvent::TurnComplete { stop_reason, usage }) => {
            assert_eq!(stop_reason, StopReason::EndOfTurn);
            // A snapshot here would be applied last-write-wins over the summed
            // deltas, replacing the run's real cost with the final request's.
            assert!(usage.is_none(), "got {usage:?}");
        }
        other => panic!("expected TurnComplete, got {other:?}"),
    }
}

#[test]
fn agent_end_announcing_a_retry_is_not_terminal() {
    let line = r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"error"}],"willRetry":true}"#;
    assert!(parse_pi_event(line).is_none());
}

#[test]
fn agent_settled_is_not_terminal() {
    assert!(parse_pi_event(r#"{"type":"agent_settled"}"#).is_none());
}

#[test]
fn stop_reasons_map_to_the_domain_vocabulary() {
    let cases = [
        ("stop", StopReason::EndOfTurn),
        ("aborted", StopReason::Cancelled),
        ("length", StopReason::MaxTokens),
        ("maxTokens", StopReason::MaxTokens),
        ("error", StopReason::Error),
        ("toolUse", StopReason::EndOfTurn),
        ("pending", StopReason::EndOfTurn),
    ];
    for (raw, want) in cases {
        let line = format!(
            r#"{{"type":"agent_end","messages":[{{"stopReason":"{raw}"}}],"willRetry":false}}"#
        );
        match parse_pi_event(&line) {
            Some(AgentEvent::TurnComplete { stop_reason, .. }) => {
                assert_eq!(stop_reason, want, "stopReason {raw}")
            }
            other => panic!("expected TurnComplete for {raw}, got {other:?}"),
        }
    }
}

#[test]
fn retry_and_compaction_surface_as_breadcrumbs() {
    let retry = r#"{"type":"auto_retry_start","attempt":2,"maxAttempts":5,"errorMessage":"429 rate limited"}"#;
    match parse_pi_event(retry) {
        Some(AgentEvent::Text { delta }) => {
            assert!(delta.contains("2/5"), "got {delta:?}");
            assert!(delta.contains("429 rate limited"), "got {delta:?}");
        }
        other => panic!("expected Text breadcrumb, got {other:?}"),
    }

    match parse_pi_event(r#"{"type":"compaction_start","reason":"token threshold"}"#) {
        Some(AgentEvent::Text { delta }) => assert!(delta.contains("token threshold")),
        other => panic!("expected Text breadcrumb, got {other:?}"),
    }
}

#[test]
fn unknown_and_malformed_lines_are_dropped() {
    assert!(parse_pi_event(r#"{"type":"future_event","x":1}"#).is_none());
    assert!(parse_pi_event(r#"{"no_type":true}"#).is_none());
    assert!(parse_pi_event("not json").is_none());
    assert!(parse_pi_event("").is_none());
}

// ── Golden-transcript replay ───────────────────────────────────────────

#[test]
fn golden_text_only_transcript_replays_to_expected_event_sequence() {
    let events = replay(TEXT_ONLY);

    assert_eq!(events.len(), 3, "got {events:?}");
    assert!(matches!(&events[0], AgentEvent::Text { delta } if delta.as_str() == "pong"));
    match &events[1] {
        AgentEvent::UsageDelta(u) => {
            assert_eq!(u.input_tokens, 480);
            assert_eq!(u.output_tokens, 5);
        }
        other => panic!("expected UsageDelta, got {other:?}"),
    }
    assert!(matches!(
        events[2],
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: None
        }
    ));
}

#[test]
fn golden_tool_call_transcript_replays_to_expected_event_sequence() {
    let events = replay(MULTI_TURN);

    assert_eq!(events.len(), 6, "got {events:?}");
    match &events[0] {
        AgentEvent::ToolCall {
            action,
            target,
            tool_call_id,
            ..
        } => {
            assert_eq!(*action, ActionKind::Read);
            assert_eq!(target, ".");
            // pi concatenates the provider's call id and its own with a pipe;
            // the whole string is the correlation key for the update below.
            assert!(tool_call_id.starts_with("call_kO6nddrz5JTTNWNER4fwnmWU|"));
        }
        other => panic!("expected ToolCall, got {other:?}"),
    }
    assert!(matches!(
        &events[1],
        AgentEvent::ToolCallUpdate {
            status: ToolCallStatus::Completed,
            ..
        }
    ));
    assert!(matches!(&events[2], AgentEvent::UsageDelta(_)));
    assert!(matches!(&events[3], AgentEvent::Text { delta } if delta.as_str() == "DONE"));
    assert!(matches!(&events[4], AgentEvent::UsageDelta(_)));
    assert!(matches!(
        events[5],
        AgentEvent::TurnComplete {
            stop_reason: StopReason::EndOfTurn,
            usage: None
        }
    ));
}

/// Consulting this would mean the accumulator fell back to a derived estimate
/// instead of pi's own reported cost.
struct NeverPriced;

impl PricingTable for NeverPriced {
    fn price_for(&self, model: &str) -> Option<ModelPrice> {
        panic!("pricing consulted for {model}, but pi reports cost itself");
    }
    fn context_window(&self, model: &str) -> Option<u64> {
        panic!("context_window consulted for {model}");
    }
    fn known_models(&self) -> Vec<String> {
        panic!("known_models consulted");
    }
}

/// The end-to-end justification for `AgentEvent::UsageDelta`: the numbers
/// below are the two `turn_end`s of a real captured run, not a construction.
#[test]
fn golden_multi_turn_usage_sums_instead_of_maxing() {
    let mut acc = UsageAccumulator::new(Some("openai-codex/gpt-5.6-luna".to_string()));
    for event in replay(MULTI_TURN) {
        acc.ingest_event(&event);
    }
    acc.finalize(&NeverPriced);

    let output_tokens = acc.tokens() as u64 - acc.input_tokens();
    assert_eq!(output_tokens, 18 + 5, "18 and 5 summed, not maxed to 18");
    assert!(
        (acc.cost_usd() - 0.002303).abs() < 1e-9,
        "0.001166 and 0.001137 summed, not last-write-wins to 0.001137; got {}",
        acc.cost_usd()
    );
    assert_eq!(acc.input_tokens(), 1058 + 1107);
    assert_eq!(acc.tokens(), 1058 + 18 + 1107 + 5);
    assert!(acc.finished());
}

// ── Session-id capture ─────────────────────────────────────────────────

#[test]
fn the_session_header_is_the_only_line_that_yields_a_session_id() {
    let mut captured = Vec::new();
    for line in MULTI_TURN.lines().filter(|l| !l.trim().is_empty()) {
        let v: serde_json::Value =
            serde_json::from_str(line).unwrap_or_else(|e| panic!("fixture line is not JSON: {e}"));
        if let Some(sid) = session_id_from_line(&v) {
            captured.push(sid.to_string());
        }
    }
    assert_eq!(captured, vec!["019fbb89-d498-7012-8fd8-fcdd6b505378"]);
}

#[test]
fn a_bare_id_outside_the_session_header_is_not_a_session_id() {
    for line in [
        r#"{"type":"message_start","id":"msg-1"}"#,
        r#"{"type":"tool_execution_start","id":"tool-abc","toolName":"read"}"#,
        r#"{"type":"agent_end","id":"agent-1","willRetry":false}"#,
    ] {
        let v: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(session_id_from_line(&v), None, "leaked from {line}");
    }
}

// ── Arg builder ────────────────────────────────────────────────────────

fn ctx() -> AgentContext {
    AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "pi".into(),
        args: vec![],
        env: HashMap::new(),
        cwd: ".".into(),
        model: None,
        effort: None,
        title: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: PermissionProfile::all_allow(),
        bare_mode: false,
        tool_allowlist: None,
        max_turns: None,
        max_budget_usd: None,
    }
}

fn arg_pair<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

#[test]
fn args_open_with_json_mode_and_never_approve() {
    let args = build_pi_args(&ctx(), None, "implement the thing");
    assert_eq!(args[0], "--mode");
    assert_eq!(args[1], "json");
    // Without `-na` a fleet run's behaviour depends on which projects the user
    // happened to trust interactively in `~/.pi/agent/trust.json`.
    assert!(args.contains(&"-na".to_string()), "got {args:?}");
    assert_eq!(args.last().map(String::as_str), Some("implement the thing"));
    assert!(!args.contains(&"--session".to_string()));
}

/// Dropping context files would cost prompt-cache reuse (AGENTS.md is
/// byte-identical across a feature's worktrees) *and* strip the project
/// constitution out of every agent turn.
#[test]
fn args_never_suppress_context_files() {
    let mut c = ctx();
    c.bare_mode = true;
    let args = build_pi_args(&c, None, "go");
    assert!(!args.contains(&"-nc".to_string()), "got {args:?}");
    assert!(
        !args.contains(&"--no-context-files".to_string()),
        "got {args:?}"
    );
}

#[test]
fn args_resume_the_captured_session() {
    let args = build_pi_args(&ctx(), Some("019fbb89-d498-7012-8fd8-fcdd6b505378"), "next");
    assert_eq!(
        arg_pair(&args, "--session"),
        Some("019fbb89-d498-7012-8fd8-fcdd6b505378")
    );
}

#[test]
fn args_model_and_title_are_passed_through() {
    let mut c = ctx();
    c.model = Some("openai-codex/gpt-5.6-luna".into());
    c.title = Some("Add pi adapter".into());
    let args = build_pi_args(&c, None, "go");
    assert_eq!(
        arg_pair(&args, "--model"),
        Some("openai-codex/gpt-5.6-luna")
    );
    assert_eq!(arg_pair(&args, "--name"), Some("Add pi adapter"));
}

/// Every emitted value is pinned literally rather than recomputed from
/// `clamp_for`, which would assert the implementation against itself.
#[test]
fn args_effort_is_clamped_before_it_is_emitted() {
    for (level, want) in [
        (EffortLevel::Low, "low"),
        (EffortLevel::Medium, "medium"),
        (EffortLevel::High, "high"),
        (EffortLevel::XHigh, "xhigh"),
        (EffortLevel::Max, "max"),
    ] {
        let mut c = ctx();
        c.effort = Some(level);
        let args = build_pi_args(&c, None, "go");
        assert_eq!(arg_pair(&args, "--thinking"), Some(want), "level {level}");
    }
    assert_eq!(EffortLevel::supported_for(AgentKind::Pi).len(), 5);
}

#[test]
fn args_omit_thinking_when_effort_is_unset() {
    let args = build_pi_args(&ctx(), None, "go");
    assert!(!args.contains(&"--thinking".to_string()), "got {args:?}");
}

#[test]
fn args_exclude_the_tools_the_permission_profile_denies() {
    let mut c = ctx();
    c.permissions = StepCapability::ReadOnly.base_profile();
    let args = build_pi_args(&c, None, "review");
    assert_eq!(arg_pair(&args, "-xt"), Some("edit,write,bash"));

    c.permissions = PermissionProfile {
        read_fs: Access::Deny,
        ..PermissionProfile::all_allow()
    };
    let args = build_pi_args(&c, None, "review");
    assert_eq!(arg_pair(&args, "-xt"), Some("read,grep,find,ls"));
}

/// pi ships no webfetch or websearch tool, so `network: Deny` has nothing to
/// exclude. Faking one here would report an enforcement that never happened.
#[test]
fn denied_network_alone_excludes_nothing() {
    let profile = PermissionProfile {
        network: Access::Deny,
        ..PermissionProfile::all_allow()
    };
    assert!(excluded_tools_for(&profile).is_empty());

    let mut c = ctx();
    c.permissions = profile;
    let args = build_pi_args(&c, None, "go");
    assert!(!args.contains(&"-xt".to_string()), "got {args:?}");
}

#[test]
fn args_empty_allowlist_is_emitted_as_an_empty_string() {
    let mut c = ctx();
    c.tool_allowlist = Some(vec![]);
    let args = build_pi_args(&c, None, "go");
    assert_eq!(arg_pair(&args, "-t"), Some(""));

    c.tool_allowlist = Some(vec!["read".into(), "grep".into()]);
    let args = build_pi_args(&c, None, "go");
    assert_eq!(arg_pair(&args, "-t"), Some("read,grep"));
}

/// The one non-empty allowlist the orchestrator sets is the finalize turn's,
/// spelled in claude-code's vocabulary. Forwarded verbatim it names no pi tool
/// at all, and the turn that exists to read a truncated diff runs blind.
#[test]
fn args_translate_the_allowlist_into_pi_tool_names() {
    let mut c = ctx();
    c.tool_allowlist = Some(vec!["Read".into(), "Grep".into(), "Glob".into()]);
    let args = build_pi_args(&c, None, "summarize");
    assert_eq!(arg_pair(&args, "-t"), Some("read,grep,find"));
}

/// Dropping the flag rather than emitting `-t ""`: an ask pi cannot express
/// degrades to the full tool set, never to no tools at all.
#[test]
fn args_drop_an_allowlist_naming_no_pi_tool() {
    let mut c = ctx();
    c.tool_allowlist = Some(vec!["WebFetch".into(), "TodoWrite".into()]);
    let args = build_pi_args(&c, None, "go");
    assert!(!args.contains(&"-t".to_string()), "got {args:?}");
}

#[test]
fn args_bare_mode_pins_the_static_prefix() {
    let mut c = ctx();
    c.bare_mode = true;
    let args = build_pi_args(&c, None, "go");
    for flag in [
        "--no-extensions",
        "--no-skills",
        "--no-prompt-templates",
        "--no-themes",
    ] {
        assert!(
            args.contains(&flag.to_string()),
            "missing {flag} in {args:?}"
        );
    }

    let args = build_pi_args(&ctx(), None, "go");
    assert!(
        !args.contains(&"--no-extensions".to_string()),
        "got {args:?}"
    );
}

// ── Model listing ──────────────────────────────────────────────────────

#[test]
fn model_table_rows_become_provider_qualified_ids() {
    let output = "\
provider      model                context  max-out  thinking  images
openai-codex  gpt-5.3-codex-spark  128K     128K     yes       no
openai-codex  gpt-5.6-luna         272K     128K     yes       yes
";
    assert_eq!(
        parse_pi_model_table(output),
        vec![
            "openai-codex/gpt-5.3-codex-spark".to_string(),
            "openai-codex/gpt-5.6-luna".to_string()
        ]
    );
}

/// With no provider authenticated pi prints prose on this channel. Parsing it
/// as models would offer the user names that fail at runtime with "Model not
/// found" — a static fallback list has the same defect.
#[test]
fn model_listing_prose_yields_no_models() {
    let output = "\
No models available. Use /login to authenticate a provider.
See https://example.invalid/docs/models for the list.
provider      model                context  max-out  thinking  images
";
    assert!(parse_pi_model_table(output).is_empty());
}
