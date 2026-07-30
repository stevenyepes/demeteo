//! The precedence chain's project-workflow tier, reached with values only.
//!
//! Before the overlays moved here, a regression in either was observable only
//! by launching a feature and reading what the agent was actually told.

use super::*;
use crate::domain::ids::{ProjectId, StepId, WorkflowId};
use crate::domain::models::{EffortLevel, WorktreeStrategy};

fn settings() -> ProjectSettings {
    ProjectSettings {
        project_id: ProjectId::from("p-1".to_string()),
        worktree_strategy: WorktreeStrategy {
            default_branch: "main".to_string(),
            branch_prefix: "demeteo/features/".to_string(),
            test_command: None,
            build_command: None,
            coverage_command: None,
            conventions_file: None,
            pr_template: None,
            harnesses: None,
            validation_gates: None,
            prepare_command: None,
            extra_writable_paths: Vec::new(),
        },
        conflict_policy: "manual".to_string(),
        feature_lifecycle: "keep".to_string(),
        default_agent_kind: Some("opencode".to_string()),
        default_model: Some("project-model".to_string()),
        default_effort: Some(EffortLevel::Low),
        default_loop_iterations: None,
        default_max_budget_usd: None,
        artifact_subdir: "artifacts/".to_string(),
        commit_artifacts: false,
    }
}

fn step(id: &str) -> StepConfig {
    StepConfig {
        id: StepId::from(id.to_string()),
        kind: "agent".to_string(),
        title: id.to_string(),
        agent_kind: Some("authored".to_string()),
        model: Some("authored-model".to_string()),
        effort: Some(EffortLevel::High),
        prompt_template: None,
        rework_prompt_template: None,
        on_failure: None,
        max_iterations: None,
        artifacts: None,
        verifier: None,
        ..StepConfig::default()
    }
}

fn override_row(
    step_id: Option<&str>,
    agent_kind: Option<&str>,
    model: Option<&str>,
    effort: Option<EffortLevel>,
) -> ProjectWorkflowOverride {
    ProjectWorkflowOverride {
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: WorkflowId::from("wf-1".to_string()),
        step_id: step_id.map(|s| s.to_string()),
        agent_kind: agent_kind.map(|s| s.to_string()),
        model: model.map(|s| s.to_string()),
        effort,
    }
}

// ── The workflow-level row ───────────────────────────────────────────────────

/// It becomes the effective project default, which is what leaves
/// `resolve_agent_model` untouched: every more specific tier still wins,
/// because nothing about the chain below it changed.
#[test]
fn a_workflow_level_row_becomes_the_effective_project_default() {
    let mut s = settings();
    overlay_workflow_defaults(
        &mut s,
        &[override_row(
            None,
            Some("claude-code"),
            Some("wf-model"),
            Some(EffortLevel::Medium),
        )],
    );

    assert_eq!(s.default_agent_kind.as_deref(), Some("claude-code"));
    assert_eq!(s.default_model.as_deref(), Some("wf-model"));
    assert_eq!(s.default_effort, Some(EffortLevel::Medium));
}

/// The regression the `is_some()` guards exist to prevent. A row that sets
/// nothing must clear nothing — `None` on a field means *inherit*, and a bare
/// assignment would wipe the project's own default instead.
#[test]
fn a_row_with_every_field_unset_is_a_no_op() {
    let mut s = settings();
    overlay_workflow_defaults(&mut s, &[override_row(None, None, None, None)]);

    assert_eq!(s.default_agent_kind.as_deref(), Some("opencode"));
    assert_eq!(s.default_model.as_deref(), Some("project-model"));
    assert_eq!(s.default_effort, Some(EffortLevel::Low));
}

/// Each field overlays on its own. A row that names only an agent must not
/// drag the model along with it.
#[test]
fn each_field_overlays_independently() {
    let mut s = settings();
    overlay_workflow_defaults(&mut s, &[override_row(None, Some("hermes"), None, None)]);

    assert_eq!(s.default_agent_kind.as_deref(), Some("hermes"));
    assert_eq!(
        s.default_model.as_deref(),
        Some("project-model"),
        "an agent-only row must leave the model alone"
    );
    assert_eq!(s.default_effort, Some(EffortLevel::Low));
}

/// Only the row with no `step_id` is the workflow-level one. A step row
/// reaching the project defaults would apply one step's choice to every step
/// in the run.
#[test]
fn a_step_scoped_row_never_touches_the_project_defaults() {
    let mut s = settings();
    overlay_workflow_defaults(
        &mut s,
        &[override_row(
            Some("implement"),
            Some("hermes"),
            Some("step-model"),
            Some(EffortLevel::High),
        )],
    );

    assert_eq!(s.default_agent_kind.as_deref(), Some("opencode"));
    assert_eq!(s.default_model.as_deref(), Some("project-model"));
    assert_eq!(s.default_effort, Some(EffortLevel::Low));
}

// ── The per-step rows ────────────────────────────────────────────────────────

/// A step row is baked onto its own step and beats the workflow author's
/// value there. The step beside it keeps what the author wrote.
#[test]
fn a_step_row_beats_the_authors_value_on_that_step_alone() {
    let mut steps = vec![step("spec"), step("implement")];
    bake_step_overrides(
        &mut steps,
        &[override_row(
            Some("implement"),
            Some("hermes"),
            Some("step-model"),
            Some(EffortLevel::Medium),
        )],
    );

    assert_eq!(steps[1].agent_kind.as_deref(), Some("hermes"));
    assert_eq!(steps[1].model.as_deref(), Some("step-model"));
    assert_eq!(steps[1].effort, Some(EffortLevel::Medium));

    assert_eq!(steps[0].agent_kind.as_deref(), Some("authored"));
    assert_eq!(steps[0].model.as_deref(), Some("authored-model"));
    assert_eq!(steps[0].effort, Some(EffortLevel::High));
}

/// Same guard, on the other overlay: an all-`None` step row must not blank
/// the author's agent, model and effort.
#[test]
fn a_step_row_with_every_field_unset_is_a_no_op() {
    let mut steps = vec![step("implement")];
    bake_step_overrides(
        &mut steps,
        &[override_row(Some("implement"), None, None, None)],
    );

    assert_eq!(steps[0].agent_kind.as_deref(), Some("authored"));
    assert_eq!(steps[0].model.as_deref(), Some("authored-model"));
    assert_eq!(steps[0].effort, Some(EffortLevel::High));
}

/// Overrides are project-scoped and outlive the workflow versions they were
/// written against, so a row naming a step this run does not contain is
/// ignored rather than an error.
#[test]
fn a_row_naming_an_absent_step_is_ignored() {
    let mut steps = vec![step("spec")];
    bake_step_overrides(
        &mut steps,
        &[override_row(
            Some("deleted-step"),
            Some("hermes"),
            None,
            None,
        )],
    );

    assert_eq!(steps[0].agent_kind.as_deref(), Some("authored"));
}

/// The workflow-level row is not a step row: `bake_step_overrides` must skip
/// it, or it would silently bake the workflow default onto whichever step
/// happens to be named by an empty id.
#[test]
fn the_workflow_level_row_is_skipped_by_the_step_pass() {
    let mut steps = vec![step("spec")];
    bake_step_overrides(
        &mut steps,
        &[override_row(None, Some("hermes"), None, None)],
    );

    assert_eq!(steps[0].agent_kind.as_deref(), Some("authored"));
}
