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
        title: None,
        agent_exec: Arc::new(StubAgentExec),
        exec: Arc::new(StubExec),
        permissions: PermissionProfile::all_allow(),
        bare_mode: false,
    }
}

#[test]
fn args_no_resume_when_session_id_missing() {
    let args = build_hermes_args(&ctx_for_test(), None);
    assert!(!args.contains(&"--resume".to_string()), "got {args:?}");
}

#[test]
fn args_resume_emitted_when_captured_session_id_set() {
    let args = build_hermes_args(&ctx_for_test(), Some("hermes-sess-99"));
    let resume_idx = args
        .iter()
        .position(|a| a == "--resume")
        .expect("--resume should be present");
    assert_eq!(args[resume_idx + 1], "hermes-sess-99");
}

#[test]
fn args_run_format_json_always_present() {
    let args = build_hermes_args(&ctx_for_test(), None);
    assert!(args.contains(&"run".to_string()));
    assert!(args.contains(&"--format".to_string()));
    assert!(args.contains(&"json".to_string()));
}

#[test]
fn args_model_passed_through() {
    let args = build_hermes_args(&ctx_for_test(), None);
    let model_idx = args
        .iter()
        .position(|a| a == "--model")
        .expect("--model should be present");
    assert_eq!(args[model_idx + 1], "claude-sonnet-4");
}
