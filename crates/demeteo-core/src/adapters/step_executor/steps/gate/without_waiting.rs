//! The answer a gate can have before anyone is asked.

use crate::domain::gate::review_contract::ReviewStanding;
use crate::domain::gate_autonomy::{policy_answer, GateAutonomy, PolicyAnswer};
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

/// Why nothing answered the gate, so a person has to.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct MustAsk {
    /// Set when the project's autonomy would have approved the gate on a
    /// clean review and the review did not allow it, in words for the person
    /// deciding. `None` for a gate no policy would have approved.
    pub held_by_review: Option<&'static str>,
}

/// The decision this gate already has, or the one the project's autonomy
/// gives it, or [`MustAsk`] when a person has to be asked.
///
/// **A recorded answer is read first, and wins.** It is what survives a
/// restart; the policy writes over the row, so checking it first would turn a
/// person's recorded refusal into an approval.
///
/// **A policy approval counts only once it reads back.** A run that went on
/// from an approval it could not re-read would find no answer after a restart
/// — [`MustAsk`] sends the caller to its waiter, whose poll picks the row up
/// if the write did land.
///
/// `review` is called only when the policy's answer turns on it, so a gate
/// that needs no review reads no artifact.
pub(crate) fn decided_without_waiting(
    gates: &dyn GateRepository,
    step_execution_id: &StepExecutionId,
    autonomy: GateAutonomy,
    dangerous: bool,
    review: impl FnOnce() -> ReviewStanding,
    now_ms: i64,
) -> Result<Unasked, MustAsk> {
    if let Some(recorded) = gates.latest_for_step(step_execution_id).ok().flatten() {
        if recorded.decision.is_some() {
            return Ok(Unasked::Recorded(recorded));
        }
    }
    match policy_answer(autonomy, dangerous) {
        PolicyAnswer::Ask => return Err(MustAsk::default()),
        PolicyAnswer::Approve => {}
        PolicyAnswer::ApproveIfReviewClean => {
            if let Some(reason) = review().held_reason() {
                return Err(MustAsk {
                    held_by_review: Some(reason),
                });
            }
        }
    }
    gates
        .approve_by_policy(step_execution_id, now_ms)
        .ok()
        .and_then(|()| gates.latest_for_step(step_execution_id).ok().flatten())
        .filter(|row| row.auto_approved && row.decision.as_deref() == Some("approve"))
        .map(Unasked::ApprovedByPolicy)
        .ok_or_else(MustAsk::default)
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/gate_without_waiting.rs"]
mod tests;
