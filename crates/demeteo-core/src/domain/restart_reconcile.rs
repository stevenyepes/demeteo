//! What a step left mid-run by a dead process becomes on restart.
//!
//! A hard kill — OOM, crash, force-quit, an update that restarted the app —
//! leaves rows describing work no process is doing any more: a step that says
//! `running` with nothing running it, a gate that says `awaiting_gate` with no
//! driver left to receive the answer, a `pending` step under a feature that
//! already ended. Nothing will ever move those rows again, so the next process
//! to start has to decide what each of them meant.
//!
//! The decision is two arms and a sentence each, and it used to sit six levels
//! deep inside the watchdog's nested loops — reachable only by standing up a
//! `DagStepExecutor` with `projects`, `features`, `gates`, `notif`,
//! `subtask_runs` and `remote_run_mirror` all stubbed, which is why none of
//! these strings and none of the synthesise rule was asserted anywhere.
//!
//! Everything here is synchronous and total. The adapter keeps the
//! choreography: enumerating projects and features, skipping runner-owned
//! shadows (C4.2), closing stale `subtask_runs` rows, writing the patch,
//! creating the gate row, and emitting the events.

/// What one step of a still-live feature becomes when the process that was
/// driving it is gone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterruptedStep {
    /// The `error_message` the row is left carrying. It is what the user reads
    /// in the timeline, and it names which of the two things was interrupted —
    /// the step's own work, or the wait for a human.
    pub message: String,
    /// Whether the reconciliation must also create an undecided
    /// [`GateDecision`](crate::domain::models::GateDecision) row for this step.
    ///
    /// Recovery ends by asking the user what to do with the interrupted step,
    /// and that prompt is backed by a pending gate row. A step that was already
    /// `awaiting_gate` has one — it is why it was waiting — so synthesising a
    /// second would overwrite the real decision's slot. A `running` step has
    /// none, so one is synthesised for it, and that is the only reason the two
    /// arms differ in anything but wording.
    pub synthesise_gate_decision: bool,
}

/// Whether `step_status` describes work a dead process was in the middle of,
/// and what it becomes if so.
///
/// Only the two in-flight statuses answer. Everything else is either terminal
/// (nothing to reconcile) or `pending` — which is *not* interrupted work: the
/// scheduler simply never reached it, and it stays runnable when the feature
/// resumes. See [`orphaned_by_feature_end`] for the one case where a `pending`
/// step is not runnable.
pub fn interrupted_by_restart(step_status: &str) -> Option<InterruptedStep> {
    match step_status {
        "awaiting_gate" => Some(InterruptedStep {
            message: "Gate interrupted by system restart".to_string(),
            synthesise_gate_decision: false,
        }),
        "running" => Some(InterruptedStep {
            message: "Step interrupted by system restart".to_string(),
            synthesise_gate_decision: true,
        }),
        _ => None,
    }
}

/// Whether a feature has reached a status its steps can never advance past.
///
/// The reconciliation reads this twice — once to decide whether a feature's
/// step rows are worth reading at all, and once per step inside
/// [`orphaned_by_feature_end`] — so it is spelled here rather than at either
/// call site.
pub fn feature_ended(feature_status: &str) -> bool {
    matches!(feature_status, "cancelled" | "failed")
}

/// A `pending` step under a feature that already ended: it can never advance,
/// so it is closed rather than left spinning.
///
/// This is the other half of a hard kill. Where [`interrupted_by_restart`]
/// handles rows the dead process was working on, this handles the rows behind
/// them — steps the scheduler had not reached when the feature was cancelled
/// or failed. Under a *live* feature the identical row is perfectly healthy,
/// which is why the feature's status is an input rather than something the
/// caller may assume it has already checked.
pub fn orphaned_by_feature_end(feature_status: &str, step_status: &str) -> Option<String> {
    if feature_ended(feature_status) && step_status == "pending" {
        return Some("Step orphaned: feature ended before step ran".to_string());
    }
    None
}

#[cfg(test)]
#[path = "../../tests/domain/restart_reconcile.rs"]
mod tests;
