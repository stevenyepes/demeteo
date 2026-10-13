use super::*;
use crate::domain::ids::{FeatureId, StepId};

fn step(id: &str, index: u32, kind: &str, status: &str) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from(id.to_string()),
        feature_id: FeatureId::from("f".to_string()),
        step_id: StepId::from(format!("s-{id}")),
        step_index: index,
        step_kind: kind.to_string(),
        status: status.to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 7,
    }
}

fn none() -> HashSet<StepExecutionId> {
    HashSet::new()
}

#[test]
fn only_a_gate_step_awaiting_a_decision_counts() {
    let steps = [
        step("a", 0, "agent", "awaiting_gate"),
        step("g1", 1, "gate", "completed"),
        step("g2", 2, "gate", "running"),
    ];
    assert!(awaiting_unrecorded_gate(&steps, &none()).is_none());
}

#[test]
fn the_earliest_awaiting_gate_is_named_by_its_step_execution() {
    let steps = [
        step("late", 4, "gate", "awaiting_gate"),
        step("early", 2, "gate", "awaiting_gate"),
    ];
    let gate = awaiting_unrecorded_gate(&steps, &none()).expect("a gate is parked");
    assert_eq!(gate.step_execution_id.as_str(), "early");
    assert_eq!(gate.decision, None);
    assert_eq!(gate.created_at, 7);
}

#[test]
fn a_gate_with_a_local_row_is_left_to_that_row() {
    let steps = [step("g", 1, "gate", "awaiting_gate")];
    let recorded = HashSet::from([StepExecutionId::from("g".to_string())]);
    assert!(awaiting_unrecorded_gate(&steps, &recorded).is_none());
}
