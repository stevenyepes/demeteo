//! Parking a step for a human, and what their answer means.
//!
//! Two situations reach a human mid-run with the same shape — *this step
//! cannot proceed, no retry changes that, and a person can decide*:
//!
//! - a node interrupted by a restart whose workspace moved underneath it
//!   (the resume fingerprint guard), and
//! - a rework cycle whose producer emitted no tickets because it found
//!   nothing an implementation ticket could fix.
//!
//! Before this, only the first could park; the second returned
//! `NonRetryable` and ended the run. Its own prompt calls emitting nothing
//! *"a supported answer, not a failure: it ends the run and puts the
//! decision in front of a human"* — and ending the run put it in front of
//! nobody, leaving the reason in a database column.
//!
//! The park mechanics are shared (`adapters::step_executor::gate_park`);
//! what differs is the reason and whether `redirect` means anything, which
//! is what [`HumanPark`] carries.

use crate::domain::ids::StepId;
use crate::domain::models::GateDecision;

/// A request to stop and ask a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanPark {
    /// What the human reads **before** deciding.
    ///
    /// The load-bearing field. A park whose reason only surfaces after the
    /// answer — which is what the resume guard did, using its mismatch
    /// string solely to build the decline message — asks someone to choose
    /// blind.
    pub reason: String,
    /// Where `redirect` sends the run, or `None` when redirect is not a
    /// meaningful answer to this park.
    ///
    /// The resume guard has no target: its only question is "safe to re-run
    /// here?", and there is no earlier step that makes a moved workspace
    /// safe. A zero-ticket rework does have one — the producer that emitted
    /// nothing can be told what to emit instead.
    pub redirect_to: Option<StepId>,
}

/// What the run does with the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParkResolution {
    /// Approved: the step is done. For a zero-ticket rework that is
    /// literally true — there was nothing to implement.
    Complete,
    Redirect {
        target: StepId,
        feedback: String,
    },
    Fail(String),
    /// The run was cancelled while parked.
    Cancelled,
}

/// Resolve a parked step against the decision that arrived.
///
/// `None` means the waiter woke on cancellation rather than an answer.
///
/// Total and synchronous, over the two values the caller already holds, so
/// the whole table is assertable without a driver. It is shared by both
/// parks deliberately: the alternative is two divergent copies of the same
/// three-way answer, and the resume guard's rules were previously spelled
/// inline where nothing could reach them.
pub fn resolve_park(park: &HumanPark, decision: Option<&GateDecision>) -> ParkResolution {
    let Some(decision) = decision else {
        return ParkResolution::Cancelled;
    };
    match decision.decision.as_deref() {
        Some("approve") => ParkResolution::Complete,
        Some("redirect") => match park.redirect_to.as_ref() {
            Some(target) => ParkResolution::Redirect {
                target: target.clone(),
                feedback: decision
                    .feedback
                    .clone()
                    .unwrap_or_else(|| park.reason.clone()),
            },
            // A redirect offered by the shared modal but meaningless for
            // this park. Failing is the honest answer and preserves the
            // resume guard's documented behaviour; silently approving
            // would resume a node the human declined to resume.
            None => ParkResolution::Fail(format!(
                "{}; user answered 'redirect', which this park cannot honour",
                park.reason
            )),
        },
        other => ParkResolution::Fail(format!(
            "{}; user answered '{}'",
            park.reason,
            other.unwrap_or("none")
        )),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/step_park.rs"]
mod step_park_tests;
