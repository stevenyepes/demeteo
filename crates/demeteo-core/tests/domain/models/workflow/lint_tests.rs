// Tests extracted from `src/domain/models/workflow.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;
use crate::domain::verifier::VerifierConfig;

fn step(id: &str, capability: StepCapability, on_failure: Option<&str>) -> StepConfig {
    StepConfig {
        effort: None,
        id: StepId::from(id.to_string()),
        kind: "agent".into(),
        title: id.into(),
        agent_kind: None,
        model: None,
        prompt_template: None,
        on_failure: on_failure.map(|s| StepId::from(s.to_string())),
        max_iterations: None,
        artifacts: None,
        verifier: None,
        capability: Some(capability),
        allow_network: false,
        allow_shell: false,
        gate_class: None,
        task_list_from: None,
        ..Default::default()
    }
}

fn with_verifier(mut s: StepConfig) -> StepConfig {
    s.verifier = Some(VerifierConfig {
        effort: None,
        agent_kind: None,
        model: None,
        instructions: "check it".into(),
        harness_name: None,
        verdict_key: "verdict".into(),
    });
    // A properly-authored looping judge attaches the artifact it grades
    // against; give it one by default so invariant #4 (judge must not
    // grade blind) is satisfied unless a test deliberately clears it.
    if s.prompt_template.is_none() {
        s.prompt_template = Some("Grade against [attached — previous step artifact].".into());
    }
    s
}

#[test]
fn clean_pipeline_has_no_violations() {
    let steps = vec![
        step("s-plan", StepCapability::Artifacts, None),
        step("s-implement", StepCapability::Implement, None),
        with_verifier(step(
            "s-validate",
            StepCapability::Verify,
            Some("s-implement"),
        )),
    ];
    assert!(lint_workflow_steps(&steps).is_empty());
}

#[test]
fn flags_duplicate_step_ids() {
    let steps = vec![
        step("s-implement", StepCapability::Implement, None),
        step("s-implement", StepCapability::Implement, None),
    ];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("duplicate step id")),
        "{:?}",
        errors
    );
}

#[test]
fn flags_on_failure_target_that_does_not_exist() {
    let steps = vec![with_verifier(step(
        "s-validate",
        StepCapability::Verify,
        Some("s-nonexistent"),
    ))];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("does not exist")),
        "{:?}",
        errors
    );
}

#[test]
fn flags_on_failure_target_that_is_not_earlier() {
    let steps = vec![
        with_verifier(step(
            "s-validate",
            StepCapability::Verify,
            Some("s-implement"),
        )),
        step("s-implement", StepCapability::Implement, None),
    ];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("not earlier in the DAG")),
        "{:?}",
        errors
    );
}

#[test]
fn flags_self_referencing_on_failure() {
    let steps = vec![with_verifier(step(
        "s-validate",
        StepCapability::Verify,
        Some("s-validate"),
    ))];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("not earlier in the DAG")),
        "{:?}",
        errors
    );
}

#[test]
fn flags_verify_capability_on_failure_without_verifier() {
    let steps = vec![
        step("s-implement", StepCapability::Implement, None),
        step("s-smoke", StepCapability::Verify, Some("s-implement")),
    ];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("can never trigger")),
        "{:?}",
        errors
    );
}

#[test]
fn verify_capability_without_on_failure_is_fine_even_without_verifier() {
    // A verify-capability step with no retry loop at all doesn't
    // need a verifier — e.g. an advisory critic-style check whose
    // FAIL only surfaces at a human gate.
    let steps = vec![step("s-critic", StepCapability::Verify, None)];
    assert!(lint_workflow_steps(&steps).is_empty());
}

#[test]
fn implement_capability_on_failure_without_verifier_is_fine() {
    // Implement-capability steps have a different failure path (the
    // no-op-commit guard + infra errors) that doesn't require a
    // `verifier` config to be reachable.
    let steps = vec![
        step("s-diagnose", StepCapability::Artifacts, None),
        step("s-fix", StepCapability::Implement, Some("s-diagnose")),
    ];
    assert!(lint_workflow_steps(&steps).is_empty());
}

#[test]
fn on_failure_targeting_a_gate_step_is_fine() {
    // Redirecting to a `gate` step (re-request human approval) is a
    // legitimate pattern distinct from an implementation retry.
    let mut gate = step("s-gate-review", StepCapability::Artifacts, None);
    gate.kind = "gate".into();
    gate.capability = None;
    let steps = vec![
        gate,
        step(
            "s-implement",
            StepCapability::Implement,
            Some("s-gate-review"),
        ),
    ];
    assert!(lint_workflow_steps(&steps).is_empty());
}

#[test]
fn flags_looping_judge_that_attaches_no_artifact() {
    // A verifier + on_failure retry loop whose prompt references no
    // `[attached — <step>]` grades against a spec it was never given —
    // the "validate couldn't read the implementation spec" bug.
    let mut validate = with_verifier(step(
        "s-validate",
        StepCapability::Verify,
        Some("s-implement"),
    ));
    validate.prompt_template = Some("Run the harness and report pass/fail.".into());
    let steps = vec![
        step("s-implement", StepCapability::Implement, None),
        validate,
    ];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors
            .iter()
            .any(|e| e.contains("attaches no upstream artifact")),
        "{:?}",
        errors
    );
}

#[test]
fn looping_judge_with_an_attachment_is_fine() {
    let mut validate = with_verifier(step(
        "s-validate",
        StepCapability::Verify,
        Some("s-implement"),
    ));
    validate.prompt_template = Some("Spec: [attached — s-plan]. Grade the diff against it.".into());
    let steps = vec![
        step("s-implement", StepCapability::Implement, None),
        validate,
    ];
    assert!(lint_workflow_steps(&steps).is_empty());
}

#[test]
fn empty_on_failure_string_is_treated_as_unset() {
    let steps = vec![step("s-validate", StepCapability::Verify, Some(""))];
    assert!(lint_workflow_steps(&steps).is_empty());
}

// ── finalize placement ───────────────────────────────────────────────────

fn finalize_step(id: &str) -> StepConfig {
    let mut s = step(id, StepCapability::ReadOnly, None);
    s.kind = "finalize".into();
    s
}

#[test]
fn a_finalize_step_last_is_fine() {
    let steps = vec![
        step("s-implement", StepCapability::Implement, None),
        finalize_step("s-finalize"),
    ];
    assert!(lint_workflow_steps(&steps).is_empty());
}

#[test]
fn a_workflow_with_no_finalize_step_is_fine() {
    let steps = vec![step("s-implement", StepCapability::Implement, None)];
    assert!(lint_workflow_steps(&steps).is_empty());
}

/// Anything after the squash commits onto a branch that has already been
/// rewritten and published — its work would land outside the single commit
/// the reviewer actually sees.
#[test]
fn a_finalize_step_that_is_not_last_is_rejected() {
    let steps = vec![
        finalize_step("s-finalize"),
        step("s-implement", StepCapability::Implement, None),
    ];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("is not last")),
        "expected a not-last complaint, got: {errors:?}"
    );
}

/// The second squash would collapse the first one's commit and overwrite the
/// PR summary the first one authored.
#[test]
fn two_finalize_steps_are_rejected() {
    let steps = vec![
        step("s-implement", StepCapability::Implement, None),
        finalize_step("s-finalize-1"),
        finalize_step("s-finalize-2"),
    ];
    let errors = lint_workflow_steps(&steps);
    assert!(
        errors.iter().any(|e| e.contains("at most one is allowed")),
        "expected a duplicate-finalize complaint, got: {errors:?}"
    );
}

// --- `sequence` steps and their `task_list_from` source ---------------------
//
// A sequence step executes a task list some earlier step wrote. Getting the
// wiring wrong is invisible at authoring time and only surfaces at run time as
// "there is no task list to execute" — after the feature has already paid for a
// research and a spec step. The lint is what makes that a save-time error.

fn task_list_producer(id: &str) -> StepConfig {
    let mut s = step(id, StepCapability::Artifacts, None);
    s.artifacts = Some(vec![ArtifactDecl {
        name: "task-list".into(),
        capture: ArtifactCapture::LastWriteTo {
            path: "artifacts/task-list.json".into(),
        },
        mode: Default::default(),
        inline: Default::default(),
    }]);
    s
}

fn sequence_step(id: &str, from: &str) -> StepConfig {
    let mut s = step(id, StepCapability::Implement, None);
    s.kind = "sequence".into();
    s.task_list_from = Some(StepId::from(from.to_string()));
    s
}

#[test]
fn sequence_step_reading_a_task_list_from_an_earlier_producer_is_clean() {
    let steps = vec![
        task_list_producer("s-spec"),
        sequence_step("s-impl", "s-spec"),
    ];
    assert!(
        lint_workflow_steps(&steps).is_empty(),
        "{:?}",
        lint_workflow_steps(&steps)
    );
}

#[test]
fn sequence_step_pointing_at_a_missing_step_is_flagged() {
    let steps = vec![sequence_step("s-impl", "s-nope")];
    let errs = lint_workflow_steps(&steps);
    assert!(
        errs.iter().any(|e| e.contains("does not exist")),
        "{:?}",
        errs
    );
}

/// The source must be *earlier*: the artifact has to exist by the time the
/// sequence step runs.
#[test]
fn sequence_step_pointing_at_a_later_step_is_flagged() {
    let steps = vec![
        sequence_step("s-impl", "s-spec"),
        task_list_producer("s-spec"),
    ];
    let errs = lint_workflow_steps(&steps);
    assert!(
        errs.iter().any(|e| e.contains("not earlier in the DAG")),
        "{:?}",
        errs
    );
}

/// The source existing is not enough — it has to actually declare the
/// `task-list` artifact, or the step will find nothing to read at run time.
#[test]
fn sequence_step_whose_source_declares_no_task_list_is_flagged() {
    let steps = vec![
        step("s-spec", StepCapability::Artifacts, None),
        sequence_step("s-impl", "s-spec"),
    ];
    let errs = lint_workflow_steps(&steps);
    assert!(
        errs.iter().any(|e| e.contains("declares no `task-list`")),
        "{:?}",
        errs
    );
}

/// Only a sequence step executes a task list; the field is dead config anywhere
/// else, which misrepresents what the workflow does.
#[test]
fn task_list_from_on_a_non_sequence_step_is_flagged() {
    let mut agent = step("s-impl", StepCapability::Implement, None);
    agent.task_list_from = Some(StepId::from("s-spec".to_string()));
    let steps = vec![task_list_producer("s-spec"), agent];
    let errs = lint_workflow_steps(&steps);
    assert!(
        errs.iter().any(|e| e.contains("only `sequence` steps")),
        "{:?}",
        errs
    );
}

/// `parallel` is the superseded spelling of `sequence` and is still dispatched
/// to the same handler, so it must satisfy the same wiring rules rather than
/// being treated as a foreign kind.
#[test]
fn legacy_parallel_kind_is_treated_as_a_sequence_step() {
    let mut legacy = sequence_step("s-impl", "s-spec");
    legacy.kind = "parallel".into();
    assert!(legacy.is_sequence());
    assert_eq!(legacy.effective_capability(), StepCapability::Implement);

    let steps = vec![task_list_producer("s-spec"), legacy];
    assert!(
        lint_workflow_steps(&steps).is_empty(),
        "{:?}",
        lint_workflow_steps(&steps)
    );
}
