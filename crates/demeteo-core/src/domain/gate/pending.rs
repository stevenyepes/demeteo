//! Which gate a feature is parked on, when the desktop holds no
//! `gate_decisions` row for it.
//!
//! A run on a remote machine records its gate decisions on the runner; the
//! desktop mirrors only the step rows. The step's `awaiting_gate` status is
//! therefore the one thing both owners write, and the one the UI's Decide Gate
//! strip reads — a reader keyed on the decision table alone cannot see a
//! detached run's gate.

use std::collections::HashSet;

use crate::domain::ids::{GateDecisionId, StepExecutionId};
use crate::domain::models::{GateDecision, StepExecution};

pub fn awaiting_unrecorded_gate(
    steps: &[StepExecution],
    recorded: &HashSet<StepExecutionId>,
) -> Option<GateDecision> {
    steps
        .iter()
        .filter(|s| {
            s.step_kind == "gate" && s.status == "awaiting_gate" && !recorded.contains(&s.id)
        })
        .min_by_key(|s| s.step_index)
        .map(|s| GateDecision {
            id: GateDecisionId::from(format!("mirror-{}", s.id.as_str())),
            step_execution_id: s.id.clone(),
            decision: None,
            feedback: None,
            created_at: s.updated_at,
            auto_approved: false,
        })
}

#[cfg(test)]
#[path = "../../../tests/domain/gate/pending.rs"]
mod tests;
