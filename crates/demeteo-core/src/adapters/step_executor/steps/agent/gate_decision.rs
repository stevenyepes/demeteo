//! What the gates on this run have said, for a later agent step's prompt.
//!
//! Two questions, deliberately in one module. The singular pair
//! (`{{gate_decision}}` / `{{gate_feedback}}`) steers the step immediately
//! after a gate; the log ([`gate_decision_log`]) tells a validator what a
//! human has already signed off. Splitting them across two modules is how
//! the two drift apart.

use crate::domain::gate_decision_log::{render_gate_decision_log, DecidedGate};
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::db::GateRepository;

/// Returns `(decision, feedback)` from the most recently *decided* gate step
/// for a feature.  Used to inject `{{gate_decision}}` and `{{gate_feedback}}`
/// into the next agent step's rendered prompt.
///
/// Best-effort: returns `("", "")` when no gate has been decided yet (the
/// common case for the first agent step in any workflow).
pub(crate) fn get_latest_gate_decision(
    gates: &dyn GateRepository,
    feature_id: &str,
) -> (String, String) {
    let f_id = FeatureId::from(feature_id.to_string());
    match gates.latest_decided_for_feature(&f_id) {
        Ok(Some(decided)) => (
            decided.decision.unwrap_or_default(),
            decided.feedback.unwrap_or_default(),
        ),
        _ => (String::new(), String::new()),
    }
}

/// Render the run's decided-gate history for `{{gate_decision_log}}`.
///
/// A free function over the one port it needs, not a driver method, so it
/// is reachable from a test without twenty-odd ports the code never reads
/// (AGENTS.md §3). `step_execs` is the caller's own slice — it turns a
/// `step_execution_id` into the node name a reader recognises, and costs no
/// second query.
///
/// Best-effort: a read error renders the empty block rather than failing
/// the step. A prompt is worth less without this; it is not worth nothing.
pub(crate) fn gate_decision_log(
    gates: &dyn GateRepository,
    step_execs: &[StepExecution],
    feature_id: &str,
) -> String {
    let f_id = FeatureId::from(feature_id.to_string());
    let Ok(decided) = gates.all_decided_for_feature(&f_id) else {
        return String::new();
    };
    let rows: Vec<DecidedGate<'_>> = decided
        .iter()
        .map(|d| DecidedGate {
            // An id with no matching row still names itself: a log that
            // drops a decision because its step row was pruned would be
            // silently shorter than the truth.
            step_id: step_execs
                .iter()
                .find(|se| se.id == d.step_execution_id)
                .map_or(d.step_execution_id.0.as_str(), |se| se.step_id.0.as_str()),
            decision: d.decision.as_deref().unwrap_or_default(),
            feedback: d.feedback.as_deref(),
        })
        .collect();
    render_gate_decision_log(&rows)
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/agent/gate_decision_log.rs"]
mod gate_decision_log_tests;
