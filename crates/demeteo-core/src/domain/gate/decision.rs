//! What one recorded gate decision means.

use crate::domain::ids::StepId;

/// The four fates a recorded decision resolves to.
///
/// [`Cancel`](Self::Cancel) and [`Unrecognised`](Self::Unrecognised) are
/// separate on purpose and land on **different** step outcomes: an explicit
/// refusal fails the gate, while anything the vocabulary does not contain
/// cancels the run. Collapsing them would make a typo indistinguishable from a
/// rejection, which is the thing the split exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GateVerdict {
    /// The work stands. `signal` is the reviewer's note, when they left one.
    Approve { signal: Option<String> },
    /// An explicit refusal.
    Cancel,
    /// Any other string, or no decision at all.
    Unrecognised,
    /// Send the run back to an earlier step.
    Redirect {
        /// The reviewer's guidance, captured as a memory signal — but only
        /// when it is prose. Feedback that *names a step* is an address, not
        /// advice, and storing it as guidance would teach the run nothing.
        signal: Option<String>,
        /// The same guidance, surfaced to the redirected step's prompt.
        /// Set whenever the feedback is non-empty, **including** when it
        /// names a step — the two conditions are deliberately different.
        retry_feedback: Option<String>,
    },
}

/// Read a recorded decision against the run's step list.
///
/// The asymmetry in the [`Redirect`](GateVerdict::Redirect) arm is the easiest
/// thing here to break by accident: `signal` is suppressed when the feedback
/// names a step, `retry_feedback` is not. Unifying the two conditions would
/// put a bare step id into the next agent's prompt as though it were the
/// reviewer's instruction.
pub(crate) fn classify(
    decision: Option<&str>,
    feedback: Option<&str>,
    step_ids: &[StepId],
) -> GateVerdict {
    let cleaned = feedback.map(str::trim).filter(|s| !s.is_empty());
    match decision {
        Some("approve") => GateVerdict::Approve {
            signal: cleaned.map(str::to_string),
        },
        // `reject` is the remote inbox's word for `cancel` (the
        // detached-run gate buttons are Approve / Reject). Spelled out
        // rather than left to the catch-all below, which cancels on
        // *any* unrecognised decision — so a genuine typo stays
        // distinguishable from a rejection.
        Some("cancel") | Some("reject") => GateVerdict::Cancel,
        Some("redirect") => GateVerdict::Redirect {
            signal: cleaned
                .filter(|c| !step_ids.iter().any(|id| id.0 == *c))
                .map(str::to_string),
            retry_feedback: cleaned.map(str::to_string),
        },
        _ => GateVerdict::Unrecognised,
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/gate/decision.rs"]
mod tests;
