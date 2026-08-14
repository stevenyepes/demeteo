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
fn result_error_attaches_usage_when_present() {
    // Regression: per Anthropic SDK cost-tracking docs, error result events
    // STILL carry `usage` and `total_cost_usd`. The parser must surface
    // them so the UsageAccumulator can credit tokens spent up to the
    // failure point — otherwise detached --print runs that exit with
    // `error_max_turns` / `error_during_execution` burn quota and report 0.
    let line = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"result":"hit max turns","stop_reason":"max_turns","total_cost_usd":0.42,"usage":{"input_tokens":1500,"output_tokens":900,"cache_creation_input_tokens":200,"cache_read_input_tokens":4000}}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::Error { usage, .. }) => {
            let u = usage.expect("error result event must carry parsed usage");
            assert_eq!(u.input_tokens, 1500);
            assert_eq!(u.output_tokens, 900);
            assert_eq!(u.cache_creation_input_tokens, 200);
            assert_eq!(u.cache_read_input_tokens, 4000);
            assert_eq!(u.cost_usd, Some(0.42));
        }
        other => panic!("expected Error with usage, got {other:?}"),
    }
}

#[test]
fn result_error_without_usage_emits_error_with_none_usage() {
    let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"api timeout","stop_reason":"error"}"#;
    match parse_claude_event(line) {
        Some(AgentEvent::Error { usage, .. }) => assert!(usage.is_none()),
        other => panic!("expected Error with None usage, got {other:?}"),
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
use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_runtime::AgentContext;
use std::collections::HashMap;
use std::sync::Arc;

fn ctx_for_test(bare_mode: bool) -> AgentContext {
    AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "claude".into(),
        args: vec![],
        env: HashMap::new(),
        cwd: ".".into(),
        model: Some("claude-sonnet-4".into()),
        effort: None,
        title: None,
        platform: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: PermissionProfile::all_allow(),
        bare_mode,
        tool_allowlist: None,
        max_turns: None,
        max_budget_usd: None,
    }
}

#[test]
fn args_no_resume_when_session_id_missing() {
    let args = build_claude_args(&ctx_for_test(false), None, "");
    assert!(!args.contains(&"--resume".to_string()), "got {args:?}");
}

#[test]
fn args_resume_emitted_when_captured_session_id_set() {
    let args = build_claude_args(&ctx_for_test(false), Some("sess-abc-123"), "");
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
    let with_bare = build_claude_args(&ctx_for_test(true), None, "");
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

    let without_bare = build_claude_args(&ctx_for_test(false), None, "");
    assert!(!without_bare.contains(&"--exclude-dynamic-system-prompt-sections".to_string()));
    assert!(!without_bare.contains(&"--setting-sources".to_string()));
    assert!(!without_bare.contains(&"--strict-mcp-config".to_string()));
}

#[test]
fn args_model_passed_through() {
    let args = build_claude_args(&ctx_for_test(false), None, "");
    let model_idx = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model should be present");
    assert_eq!(args[model_idx + 1], "claude-sonnet-4");
}

#[test]
fn args_print_and_dangerously_skip_always_present() {
    let args = build_claude_args(&ctx_for_test(true), Some("sess-1"), "");
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
    let with_bare = build_claude_args(&ctx_for_test(true), Some("sess-1"), "");
    assert!(
        !with_bare.contains(&"--settings".to_string()),
        "--settings must NOT be emitted: got {with_bare:?}"
    );
    let without_bare = build_claude_args(&ctx_for_test(false), None, "");
    assert!(
        !without_bare.contains(&"--settings".to_string()),
        "--settings must NOT be emitted: got {without_bare:?}"
    );
}

// ── Effort ───────────────────────────────────────────────────────────────

use crate::adapters::agent::claude_code::claude_effort_env;
use crate::domain::models::EffortLevel;

fn ctx_with_effort(effort: Option<EffortLevel>) -> AgentContext {
    AgentContext {
        effort,
        ..ctx_for_test(false)
    }
}

fn arg_pair<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == flag)
        .map(|i| args[i + 1].as_str())
}

#[test]
fn args_effort_emitted_when_resolved() {
    let args = build_claude_args(&ctx_with_effort(Some(EffortLevel::High)), None, "");
    assert_eq!(arg_pair(&args, "--effort"), Some("high"), "got {args:?}");
}

#[test]
fn args_no_effort_when_unset() {
    let args = build_claude_args(&ctx_with_effort(None), None, "");
    assert!(!args.contains(&"--effort".to_string()), "got {args:?}");
}

#[test]
fn effort_env_is_set_alongside_the_flag() {
    // `CLAUDE_CODE_EFFORT_LEVEL` outranks `--effort` and the child inherits
    // the host env, so a developer with it exported would override every run
    // unless we set it ourselves on every spawn.
    let env = claude_effort_env(Some(EffortLevel::High));
    assert_eq!(
        env.get("CLAUDE_CODE_EFFORT_LEVEL").map(String::as_str),
        Some("high")
    );
}

#[test]
fn effort_env_empty_when_unset() {
    assert!(claude_effort_env(None).is_empty());
}

#[test]
fn runtime_pins_headless_hygiene_env() {
    // Spawned claude-code children must not self-update mid-fleet or emit
    // non-essential background traffic; both switches ride on every spawn
    // via `UnifiedCliRuntime::static_env`.
    let env = crate::adapters::agent::claude_code::runtime().static_env;
    assert!(env.contains(&("DISABLE_AUTOUPDATER", "1")), "got {env:?}");
    assert!(
        env.contains(&("DISABLE_NONESSENTIAL_TRAFFIC", "1")),
        "got {env:?}"
    );
}

/// A declaration and the switch that makes it true are two facts that drift
/// apart silently. Upstream ships the PowerShell tool *alongside* Bash on a
/// progressive rollout, so an unpinned install decides this for itself and the
/// Windows platform block would then describe a shell the agent does not have.
/// Declaring `GitBash` without the opt-out is the mistake this catches.
#[test]
fn a_git_bash_declaration_pins_the_tool_that_would_contradict_it() {
    use crate::domain::models::WindowsAgentShell;
    use crate::ports::agent_runtime::AgentRuntime;

    let runtime = crate::adapters::agent::claude_code::runtime();
    if runtime.capabilities().windows_agent_shell == WindowsAgentShell::GitBash {
        assert!(
            runtime
                .windows_shell_env
                .contains(&("CLAUDE_CODE_USE_POWERSHELL_TOOL", "0")),
            "declared Git Bash but left the PowerShell tool to a rollout: {:?}",
            runtime.windows_shell_env
        );
    }
}

/// The pin reads as Windows-shaped and is not: the same tool is opt-in on Linux
/// and macOS, so shipping it unconditionally would strip it from Demeteo's
/// agents for a user who deliberately enabled it there. `static_env` is applied
/// on every spawn regardless of platform, which is exactly why this must not
/// live in it.
#[test]
fn the_powershell_opt_out_never_rides_a_non_windows_spawn() {
    let runtime = crate::adapters::agent::claude_code::runtime();
    assert!(
        !runtime
            .static_env
            .iter()
            .any(|(k, _)| *k == "CLAUDE_CODE_USE_POWERSHELL_TOOL"),
        "an unconditional pin reaches Linux and macOS: {:?}",
        runtime.static_env
    );
}

#[test]
fn init_with_full_toolset_is_not_bare() {
    // Read tools are never denied by demeteo, so a non-bare init always
    // announces them — even when Bash/Edit/WebSearch are disallowed.
    let v: serde_json::Value = serde_json::from_str(
        r#"{"type":"system","subtype":"init","session_id":"s1",
            "tools":["Read","Glob","Grep","Bash","Edit","Write"]}"#,
    )
    .unwrap();
    assert!(!crate::adapters::agent::claude_code::init_looks_bare(&v));
    // And the event itself stays dropped.
    assert!(parse_claude_event(&v.to_string()).is_none());
}

#[test]
fn init_with_bare_toolset_trips_the_canary() {
    // Bare mode ships only bash + file read/edit — no Glob/Grep. This is
    // the tripwire for the announced `--bare`-by-default flip in `-p`.
    let v: serde_json::Value = serde_json::from_str(
        r#"{"type":"system","subtype":"init","session_id":"s1",
            "tools":["Bash","Read","Edit","Write"]}"#,
    )
    .unwrap();
    assert!(crate::adapters::agent::claude_code::init_looks_bare(&v));
    // Canary is a warning, never an event that could fail the turn.
    assert!(parse_claude_event(&v.to_string()).is_none());
}

#[test]
fn init_without_tools_field_is_not_bare() {
    // An older wire format without `tools` is not evidence of bare mode.
    let v: serde_json::Value =
        serde_json::from_str(r#"{"type":"system","subtype":"init","session_id":"s1"}"#).unwrap();
    assert!(!crate::adapters::agent::claude_code::init_looks_bare(&v));
}

fn ctx_with_caps(tool_allowlist: Option<Vec<String>>, max_turns: Option<u32>) -> AgentContext {
    AgentContext {
        tool_allowlist,
        max_turns,
        ..ctx_for_test(false)
    }
}

#[test]
fn args_omit_tools_and_max_turns_by_default() {
    let args = build_claude_args(&ctx_with_caps(None, None), None, "");
    assert!(!args.contains(&"--tools".to_string()), "got {args:?}");
    assert!(!args.contains(&"--max-turns".to_string()), "got {args:?}");
    assert!(
        !args.contains(&"--max-budget-usd".to_string()),
        "got {args:?}"
    );
}

#[test]
fn args_max_budget_usd_emitted_when_set() {
    // Sub-dollar role fractions must pass through as a clean decimal, not a
    // rounded or scientific-notation string.
    let ctx = AgentContext {
        max_budget_usd: Some(0.5),
        ..ctx_for_test(false)
    };
    let args = build_claude_args(&ctx, None, "");
    let idx = args
        .iter()
        .position(|a| a == "--max-budget-usd")
        .expect("--max-budget-usd flag present");
    assert_eq!(args[idx + 1], "0.5");
}

#[test]
fn args_no_max_budget_usd_when_unset() {
    let args = build_claude_args(&ctx_for_test(false), None, "");
    assert!(
        !args.contains(&"--max-budget-usd".to_string()),
        "got {args:?}"
    );
}

#[test]
fn args_empty_allowlist_emits_empty_tools_flag() {
    // `--tools ""` strips every built-in tool definition — the shape the
    // triage classifier uses (its whole input is inlined in the prompt).
    let args = build_claude_args(&ctx_with_caps(Some(vec![]), None), None, "");
    let idx = args
        .iter()
        .position(|a| a == "--tools")
        .expect("--tools flag present");
    assert_eq!(
        args[idx + 1],
        "",
        "empty allowlist must emit an empty value"
    );
}

#[test]
fn args_allowlist_joins_tool_names() {
    let allow = Some(vec![
        "Read".to_string(),
        "Grep".to_string(),
        "Glob".to_string(),
    ]);
    let args = build_claude_args(&ctx_with_caps(allow, None), None, "");
    let idx = args.iter().position(|a| a == "--tools").unwrap();
    assert_eq!(args[idx + 1], "Read,Grep,Glob");
}

#[test]
fn args_max_turns_emitted_when_set() {
    let args = build_claude_args(&ctx_with_caps(None, Some(25)), None, "");
    let idx = args
        .iter()
        .position(|a| a == "--max-turns")
        .expect("--max-turns flag present");
    assert_eq!(args[idx + 1], "25");
}

#[test]
fn args_allowlist_coexists_with_disallowed_tools() {
    // The allowlist shrinks the definition set; --disallowedTools still
    // carries the permission profile's deny rules. Both may appear.
    let mut ctx = ctx_with_caps(Some(vec!["Read".to_string()]), Some(2));
    ctx.permissions = crate::domain::permission::resolve_profile(
        crate::domain::permission::StepCapability::ReadOnly,
        false, // no network
        false, // no shell
    );
    let args = build_claude_args(&ctx, None, "");
    assert!(args.contains(&"--tools".to_string()), "got {args:?}");
    assert!(
        args.contains(&"--disallowedTools".to_string()),
        "got {args:?}"
    );
}
