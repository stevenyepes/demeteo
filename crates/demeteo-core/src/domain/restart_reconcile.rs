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
    /// The `error_message` to write, or `None` to leave whatever the row
    /// already carries.
    ///
    /// `None` is not a missing message — it is a refusal to overwrite one.
    /// A step parked for a human keeps its *question* in `error_message`,
    /// and it is the only copy: `GateView` reads it off the row to show
    /// the person what they are being asked. Stamping a restart notice
    /// over it would leave the modal asking nothing, on exactly the path
    /// the park exists to serve. A real `gate` step loses only the notice,
    /// and its status already says it is waiting.
    pub message: Option<String>,
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
            message: None,
            synthesise_gate_decision: false,
        }),
        "running" => Some(InterruptedStep {
            message: Some("Step interrupted by system restart".to_string()),
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

/// A row left `running` by a turn that ran outside the run loop, and the
/// message it is closed with.
///
/// Everything else here is scoped to a feature the run loop owned, and that
/// scoping is exactly why this case escapes: the manual sync is only offered
/// on a feature that has already finished, so the first pass never looks at it
/// (`running`/`gated` only) and [`orphaned_by_feature_end`] never fires
/// (`cancelled`/`failed`, and only for `pending`). A killed resolver's row
/// would then read `running` for the rest of the feature's life with nothing
/// left that could move it.
///
/// The step id is the input rather than the feature's status because that is
/// what makes the rule safe to apply to *every* feature: only the reserved
/// [`MANUAL_SYNC_STEP_ID`](crate::domain::step_seed::MANUAL_SYNC_STEP_ID) row
/// is out of band, and a graph node in the same status is the first pass's to
/// decide.
pub fn abandoned_out_of_band(step_id: &str, step_status: &str) -> Option<String> {
    if step_id == crate::domain::step_seed::MANUAL_SYNC_STEP_ID && step_status == "running" {
        return Some("Sync interrupted by system restart".to_string());
    }
    None
}

#[cfg(test)]
#[path = "../../tests/domain/restart_reconcile.rs"]
mod tests;
