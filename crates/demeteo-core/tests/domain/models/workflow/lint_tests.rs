// Tests extracted from `src/domain/models/workflow.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;
use crate::domain::verifier::VerifierConfig;

fn step(id: &str, capability: StepCapability, on_failure: Option<&str>) -> StepConfig {
    StepConfig {
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
    }
}

fn with_verifier(mut s: StepConfig) -> StepConfig {
    s.verifier = Some(VerifierConfig {
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
