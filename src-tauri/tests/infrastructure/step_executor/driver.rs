use crate::domain::ids::StepId;
use crate::domain::models::{StepConfig, StepOverride};
use super::{resolve_agent_model, resolve_loop_iterations};

fn step(agent: Option<&str>, model: Option<&str>) -> StepConfig {
    StepConfig {
        id: StepId::from("s-impl".to_string()),
        kind: "agent".to_string(),
        title: "Implement".to_string(),
        agent_kind: agent.map(str::to_string),
        model: model.map(str::to_string),
        prompt_template: None,
        artifact_mode: "full".to_string(),
        on_failure: None,
        max_iterations: None,
        artifacts: None,
        verifier: None,
        capability: None,
        allow_network: false,
        allow_shell: false,
    }
}

#[test]
fn per_step_override_wins() {
    let ov = StepOverride {
        step_id: "s-impl".to_string(),
        agent_kind: Some("claude-code".to_string()),
        model: Some("claude-opus-4-8".to_string()),
    };
    let (a, m) = resolve_agent_model(
        Some(&ov),
        Some("hermes"),
        Some("feat-model"),
        &step(Some("opencode"), Some("step-model")),
        Some("opencode"),
        Some("proj-model"),
    );
    assert_eq!(a, "claude-code");
    assert_eq!(m.as_deref(), Some("claude-opus-4-8"));
}

#[test]
fn falls_through_to_workflow_then_project_then_default() {
    // No per-step, no feature-wide → workflow step value wins.
    let (a, m) = resolve_agent_model(
        None,
        None,
        None,
        &step(Some("claude-code"), None),
        Some("opencode"),
        Some("proj-model"),
    );
    assert_eq!(a, "claude-code");
    // model: step has none → project default fills it.
    assert_eq!(m.as_deref(), Some("proj-model"));

    // Nothing set anywhere → built-in opencode, no model.
    let (a2, m2) = resolve_agent_model(None, None, None, &step(None, None), None, None);
    assert_eq!(a2, "opencode");
    assert_eq!(m2, None);
}

#[test]
fn feature_wide_beats_workflow_but_loses_to_per_step() {
    let (a, _) = resolve_agent_model(
        None,
        Some("hermes"),
        None,
        &step(Some("opencode"), None),
        None,
        None,
    );
    assert_eq!(a, "hermes");
}

#[test]
fn loop_budget_precedence() {
    assert_eq!(resolve_loop_iterations(Some(7), Some(5), Some(2)), 7);
    assert_eq!(resolve_loop_iterations(None, Some(5), Some(2)), 5);
    assert_eq!(resolve_loop_iterations(None, None, Some(2)), 2);
    assert_eq!(resolve_loop_iterations(None, None, None), 3);
}
