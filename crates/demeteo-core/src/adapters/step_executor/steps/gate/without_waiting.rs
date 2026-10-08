//! The answer a gate can have before anyone is asked.

use crate::domain::gate_autonomy::{auto_approves, GateAutonomy};
use crate::domain::ids::StepExecutionId;
use crate::domain::models::GateDecision;
use crate::ports::db::GateRepository;

/// How a gate came to have an answer without parking.
#[derive(Debug)]
pub(crate) enum Unasked {
    /// A decision already on the row — a person's, or a policy approval
    /// granted before a restart.
    Recorded(GateDecision),
    /// The project's gate autonomy approved it just now.
    ApprovedByPolicy(GateDecision),
}

impl Unasked {
    pub(crate) fn decision(&self) -> &GateDecision {
        match self {
            Self::Recorded(d) | Self::ApprovedByPolicy(d) => d,
        }
    }
}

/// The decision this gate already has, or the one the project's autonomy
/// gives it, or `None` when a person has to be asked.
///
/// **A recorded answer is read first, and wins.** It is what survives a
/// restart; the policy writes over the row, so checking it first would turn a
/// person's recorded refusal into an approval.
///
/// **A policy approval counts only once it reads back.** A run that went on
/// from an approval it could not re-read would find no answer after a restart
/// — `None` sends the caller to its waiter, whose poll picks the row up if the
/// write did land.
pub(crate) fn decided_without_waiting(
    gates: &dyn GateRepository,
    step_execution_id: &StepExecutionId,
    autonomy: GateAutonomy,
    dangerous: bool,
    now_ms: i64,
) -> Option<Unasked> {
    if let Some(recorded) = gates.latest_for_step(step_execution_id).ok().flatten() {
        if recorded.decision.is_some() {
            return Some(Unasked::Recorded(recorded));
        }
    }
    if !auto_approves(autonomy, dangerous) {
        return None;
    }
    gates.approve_by_policy(step_execution_id, now_ms).ok()?;
    gates
        .latest_for_step(step_execution_id)
        .ok()
        .flatten()
        .filter(|row| row.auto_approved && row.decision.as_deref() == Some("approve"))
        .map(Unasked::ApprovedByPolicy)
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/gate_without_waiting.rs"]
mod tests;
