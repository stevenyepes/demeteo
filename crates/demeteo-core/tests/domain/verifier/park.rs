// Tests for `crates/demeteo-core/src/domain/verifier/park.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::models::GateDecision;
use crate::domain::step_park::{resolve_park, ParkResolution};

/// The shipped pipeline, not a hand-built copy: which step a redirect lands
/// on depends on its `task_list_from` and rework-template wiring.
fn standard_pipeline() -> Vec<StepConfig> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows/standard-feature-pipeline.json");
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    serde_json::from_value(raw["steps"].clone()).unwrap()
}

fn validate_idx(steps: &[StepConfig]) -> usize {
    steps.iter().position(|s| s.id.0 == "s-validate").unwrap()
}

fn gap() -> EvidenceGap {
    EvidenceGap {
        reason: "Nothing shows each new test was watched failing.".into(),
        criteria: vec!["AC6".into()],
    }
}

fn redirect(feedback: Option<&str>) -> GateDecision {
    GateDecision {
        id: "gd-syn-1".into(),
        step_execution_id: "se-1".to_string().into(),
        decision: Some("redirect".into()),
        feedback: feedback.map(str::to_string),
        created_at: 1,
    }
}

fn target_of(res: ParkResolution) -> (String, String) {
    match res {
        ParkResolution::Redirect { target, feedback } => (target.0, feedback),
        other => panic!("expected a redirect, got {other:?}"),
    }
}

#[test]
fn the_reason_names_the_criteria_and_every_answer() {
    let steps = standard_pipeline();
    let park = evidence_park(&gap(), &steps, validate_idx(&steps));
    assert!(park.reason.contains("- AC6"), "{}", park.reason);
    assert!(park.reason.contains("watched failing"), "{}", park.reason);
    assert!(park.reason.contains("Approve to waive"), "{}", park.reason);
    assert!(park.reason.contains("'s-spec'"), "{}", park.reason);
}

/// A sequence step cannot act on free text, so "send it back to implement"
/// lands where a real gate lands it: on the task-list producer.
#[test]
fn an_unaddressed_redirect_asks_the_producer_for_evidence_only() {
    let steps = standard_pipeline();
    let park = evidence_park(&gap(), &steps, validate_idx(&steps));
    let (target, feedback) = target_of(resolve_park(&park, Some(&redirect(None))));
    assert_eq!(target, "s-tickets");
    assert!(
        feedback.starts_with("A human asked for evidence only"),
        "{feedback}"
    );
    assert!(feedback.contains("AC6"), "{feedback}");
}

#[test]
fn naming_the_spec_step_sends_the_run_to_rewrite_the_criterion() {
    let steps = standard_pipeline();
    let park = evidence_park(&gap(), &steps, validate_idx(&steps));
    let (target, feedback) = target_of(resolve_park(
        &park,
        Some(&redirect(Some("s-spec: AC6 is a process rule, drop it"))),
    ));
    assert_eq!(target, "s-spec");
    assert_eq!(feedback, "s-spec: AC6 is a process rule, drop it");
}

#[test]
fn naming_the_implement_step_hops_to_its_producer() {
    let steps = standard_pipeline();
    let park = evidence_park(&gap(), &steps, validate_idx(&steps));
    let (target, _) = target_of(resolve_park(&park, Some(&redirect(Some("s-implement")))));
    assert_eq!(target, "s-tickets");
}

#[test]
fn gates_and_commands_are_not_offered_as_targets() {
    let steps = standard_pipeline();
    let named: Vec<String> = redirect_options(&steps, validate_idx(&steps))
        .into_iter()
        .map(|o| o.named.0)
        .collect();
    assert_eq!(
        named,
        vec!["s-research", "s-spec", "s-tickets", "s-implement"],
        "{named:?}"
    );
}

#[test]
fn approving_a_park_completes_the_step() {
    let steps = standard_pipeline();
    let park = evidence_park(&gap(), &steps, validate_idx(&steps));
    let mut approve = redirect(None);
    approve.decision = Some("approve".into());
    assert_eq!(
        resolve_park(&park, Some(&approve)),
        ParkResolution::Complete
    );
}

#[test]
fn a_recurring_failure_parks_with_the_validators_own_rework_target() {
    let steps = standard_pipeline();
    let idx = validate_idx(&steps);
    let failure = VerdictFailure::from_reason("AC6 unproven");
    let park = recurrence_park(&failure, &steps, idx, steps[idx].on_failure.as_ref());
    assert!(park.reason.contains("AC6 unproven"), "{}", park.reason);
    assert!(park.reason.contains("no code"), "{}", park.reason);
    let (target, feedback) = target_of(resolve_park(&park, Some(&redirect(None))));
    assert_eq!(target, "s-tickets");
    assert_eq!(feedback, park.reason);
}
