// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/driver.rs` (mirrored-tests convention). `super` = that module.

use super::resolution::{resolve_agent_model, resolve_effort, resolve_loop_iterations};
use crate::domain::ids::StepId;
use crate::domain::models::{EffortLevel, StepConfig, StepOverride};

fn step(agent: Option<&str>, model: Option<&str>) -> StepConfig {
    step_with_effort(agent, model, None)
}

fn step_with_effort(
    agent: Option<&str>,
    model: Option<&str>,
    effort: Option<EffortLevel>,
) -> StepConfig {
    StepConfig {
        effort,
        id: StepId::from("s-impl".to_string()),
        kind: "agent".to_string(),
        title: "Implement".to_string(),
        agent_kind: agent.map(str::to_string),
        model: model.map(str::to_string),
        prompt_template: None,
        on_failure: None,
        max_iterations: None,
        artifacts: None,
        verifier: None,
        capability: None,
        allow_network: false,
        allow_shell: false,
        gate_class: None,
        task_list_from: None,
        ..Default::default()
    }
}

#[test]
fn per_step_override_wins() {
    let ov = StepOverride {
        effort: None,
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

/// A per-step launch override carrying only an effort.
fn effort_override(effort: EffortLevel) -> StepOverride {
    StepOverride {
        effort: Some(effort),
        step_id: "s-impl".to_string(),
        agent_kind: None,
        model: None,
    }
}

/// AC1 — nothing configured anywhere runs at high. This is the whole of the
/// "default is high" requirement.
#[test]
fn empty_effort_chain_is_high() {
    assert_eq!(
        resolve_effort(None, None, &step(None, None), None),
        EffortLevel::High
    );
    assert_eq!(EffortLevel::DEFAULT, EffortLevel::High);
}

/// AC2, tier 4: the project default beats only the built-in fallback.
#[test]
fn project_default_effort_beats_the_builtin() {
    assert_eq!(
        resolve_effort(None, None, &step(None, None), Some(EffortLevel::Low)),
        EffortLevel::Low
    );
}

/// AC2, tier 3: the workflow step's own effort beats the project default.
#[test]
fn workflow_step_effort_beats_project_default() {
    let step = step_with_effort(None, None, Some(EffortLevel::Medium));
    assert_eq!(
        resolve_effort(None, None, &step, Some(EffortLevel::Low)),
        EffortLevel::Medium
    );
}

/// AC2, tier 2: the feature-wide run override beats the workflow step and
/// everything below it.
#[test]
fn feature_wide_effort_beats_workflow_and_project() {
    let step = step_with_effort(None, None, Some(EffortLevel::Medium));
    assert_eq!(
        resolve_effort(
            None,
            Some(EffortLevel::XHigh),
            &step,
            Some(EffortLevel::Low)
        ),
        EffortLevel::XHigh
    );
}

/// AC2, tier 1: the per-step launch override beats every tier below it.
#[test]
fn per_step_effort_override_beats_everything() {
    let step = step_with_effort(None, None, Some(EffortLevel::Medium));
    assert_eq!(
        resolve_effort(
            Some(&effort_override(EffortLevel::Max)),
            Some(EffortLevel::XHigh),
            &step,
            Some(EffortLevel::Low),
        ),
        EffortLevel::Max
    );
}

/// A `None` at a tier is "inherit", not "no effort": the chain keeps walking
/// past an override that only pins the model.
#[test]
fn a_none_tier_does_not_clobber_a_lower_one() {
    let ov = StepOverride {
        effort: None,
        step_id: "s-impl".to_string(),
        agent_kind: None,
        model: Some("some-model".to_string()),
    };
    assert_eq!(
        resolve_effort(Some(&ov), None, &step(None, None), Some(EffortLevel::Max)),
        EffortLevel::Max
    );
}

#[test]
fn loop_budget_precedence() {
    assert_eq!(resolve_loop_iterations(Some(7), Some(5), Some(2)), 7);
    assert_eq!(resolve_loop_iterations(None, Some(5), Some(2)), 5);
    assert_eq!(resolve_loop_iterations(None, None, Some(2)), 2);
    assert_eq!(resolve_loop_iterations(None, None, None), 3);
}
