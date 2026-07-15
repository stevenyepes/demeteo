//! Tests for the Hermes CLI event parser + arg builder.
//!
//! Wire format follows the production `--format json` ndjson output
//! documented at hermes-agent.nousresearch.com. Prompt-cache
//! telemetry (`cacheReadInputTokens` / `cacheCreationInputTokens`)
//! is expected on every `usage` / `usage_update` event per the
//! vendor docs; we parse both for the cost accounting path.

use crate::adapters::agent::hermes::build_hermes_args;
use crate::adapters::agent::test_stubs::{StubAgentExec, StubExec};
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_runtime::AgentContext;
use std::collections::HashMap;
use std::sync::Arc;

// ── Arg builder (token-optimization Tier 1) ────────────────────────────

fn ctx_for_test() -> AgentContext {
    AgentContext {
        thread_id: "t1".into(),
        machine_id: "local".into(),
        binary: "hermes".into(),
        args: vec![],
        env: HashMap::new(),
        cwd: ".".into(),
        model: Some("claude-sonnet-4".into()),
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

#[test]
fn args_no_resume_when_session_id_missing() {
    let args = build_hermes_args(&ctx_for_test(), None, "");
    assert!(!args.contains(&"--resume".to_string()), "got {args:?}");
}

#[test]
fn args_resume_emitted_when_captured_session_id_set() {
    let args = build_hermes_args(&ctx_for_test(), Some("hermes-sess-99"), "");
    let resume_idx = args
        .iter()
        .position(|a| a == "--resume")
        .expect("--resume should be present");
    assert_eq!(args[resume_idx + 1], "hermes-sess-99");
}

#[test]
fn args_run_format_json_always_present() {
    let args = build_hermes_args(&ctx_for_test(), None, "");
    assert!(args.contains(&"run".to_string()));
    assert!(args.contains(&"--format".to_string()));
    assert!(args.contains(&"json".to_string()));
}

#[test]
fn args_model_passed_through() {
    let args = build_hermes_args(&ctx_for_test(), None, "");
    let model_idx = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model should be present");
    assert_eq!(args[model_idx + 1], "claude-sonnet-4");
}

// ── Effort: hermes ships effort-unsupported ──────────────────────────────

use crate::adapters::agent::cli_runtime::no_effort_env;
use crate::domain::models::EffortLevel;

fn ctx_with_effort(effort: Option<EffortLevel>) -> AgentContext {
    AgentContext {
        effort,
        ..ctx_for_test()
    }
}

#[test]
fn declares_no_effort_levels() {
    // Hermes exposes reasoning effort only via `agent.reasoning_effort` in
    // `$HERMES_HOME/config.yaml` — there is no per-invocation control. The
    // empty capability set is what greys the picker out.
    assert!(crate::adapters::agent::hermes::runtime()
        .effort_levels
        .is_empty());
}

#[test]
fn args_carry_no_effort_even_when_one_is_resolved() {
    let args = build_hermes_args(&ctx_with_effort(Some(EffortLevel::High)), None, "");
    let baseline = build_hermes_args(&ctx_with_effort(None), None, "");
    assert_eq!(args, baseline, "effort must not change hermes argv");
    for flag in ["--effort", "--variant", "--reasoning-effort"] {
        assert!(!args.contains(&flag.to_string()), "got {args:?}");
    }
    assert!(
        !args.iter().any(|a| a.contains("effort")),
        "no effort value may leak into hermes argv: got {args:?}"
    );
}

#[test]
fn spawns_with_no_effort_env_var() {
    assert!(no_effort_env(Some(EffortLevel::High)).is_empty());
}
