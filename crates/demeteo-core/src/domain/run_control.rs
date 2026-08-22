//! Which actions a run refuses, and in what words.
//!
//! Three rules, each spelled inline at every call site that enforced it: a run
//! this machine does not own, a step whose status has no retry in it, and a
//! step something upstream is still working on. Nine copies across five files
//! — five of them the same sentence differing only in a trailing clause — is a
//! shape where a sixth entry point can be added without ever meeting the rule,
//! and where a wording drift between two of them is invisible until a user
//! reads both.
//!
//! The first of the three is invariant **C4.2**, and this is now its single
//! written record: a feature listed in the remote-run mirror is a read-only
//! *shadow* of a run a `demeteo-runner` owns and is still driving on another
//! machine. This machine never drives it, cancels it, retries its steps,
//! decides its gates, or replays it — every one of those would arm a second
//! engine against one run, in a worktree that only exists on the runner's box.
//!
//! Everything here is synchronous and total: it takes what the adapter
//! observed — a status string, the sibling rows, the ancestor set — and
//! returns the refusal, or `None`. `domain/` has no `async fn` anywhere in it,
//! which is what keeps that boundary honest. The adapter keeps the
//! choreography: reading the mirror, reading the step rows, resolving the
//! graph, and wrapping the returned string in whichever error type its
//! signature owes its caller.

use std::collections::HashSet;

use crate::domain::ids::StepId;
use crate::domain::models::StepExecution;

/// What a caller is trying to do to a run — the only thing that differs
/// between the five shadow refusals.
///
/// Named for the *action*, not for the entry point that performs it, because
/// the runner grows RPCs faster than the desktop grows call sites: the tail
/// each variant carries names the remote route the user should take instead,
/// and that route belongs to the action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunAction {
    /// Arm a local `ExecutionDriver` for the feature — the single recovery
    /// primitive every resume, gate decision and watchdog pass funnels into.
    Drive,
    /// Signal the run to stop.
    Cancel,
    /// Re-run one failed step.
    Retry,
    /// Apply a human's gate answer.
    DecideGate,
    /// Rewind a step and its descendants, then re-arm.
    Replay,
}

impl RunAction {
    /// The clause that follows the shared prefix. Kept as data rather than
    /// five `format!`s so the prefix cannot drift between them.
    fn refusal_tail(self) -> &'static str {
        match self {
            RunAction::Drive => {
                "this machine never drives it (decide its gates via the remote run instead)"
            }
            RunAction::Cancel => "cancel it on the runner (remote_cancel_run), not locally",
            RunAction::Retry => "this machine cannot retry its steps",
            RunAction::DecideGate => {
                "decide this gate on the runner (remote_decide_gate), not locally"
            }
            RunAction::Replay => "replay it on the runner, not here",
        }
    }
}

/// Refuse `action` on a runner-owned shadow (C4.2).
///
/// Returns the message rather than an error type: three call sites owe their
/// caller a bare `String` and two owe an `AppError::validation`, and the
/// wrapper is the one part of this that is genuinely adapter-shaped.
pub fn shadow_refusal(action: RunAction, feature_id: &str) -> String {
    format!(
        "Feature '{}' is a read-only shadow of a run owned by a demeteo-runner; {}",
        feature_id,
        action.refusal_tail()
    )
}

/// Refuse `action` on a row that reports work no workflow node did
/// ([`is_out_of_band`](crate::domain::step_seed::is_out_of_band)). `Some` is
/// the refusal.
///
/// Retry and Replay are the two actions that make their target the *pivot* of
/// a graph walk, and this row is not in the graph: `WorkflowGraph::closure`
/// answers `None` for its id, so both the rewind set and the ancestor set fall
/// back to comparing `step_index` — against `u32::MAX`. The rewind then takes
/// only this row, which the scheduler can never dispatch because it drives off
/// node ids, while *every* real node reads as an ancestor and any failed one
/// with a completed attempt is restored to `completed`. The feature is then set
/// `running` and a driver armed for it. So a finished run is rewritten as a
/// live one and nothing re-runs the resolution the user actually asked for.
///
/// The refusal is at the door rather than a widened fallback because there is
/// no graph answer to widen *to*: re-running this work is what the sync's own
/// affordance is for, and that path knows how to find the worktree.
pub fn out_of_band_refusal(action: RunAction, step_id: &str) -> Option<String> {
    if !crate::domain::step_seed::is_out_of_band(step_id) {
        return None;
    }
    Some(format!(
        "Step '{}' is an out-of-band sync, not a node of this run's workflow, \
         so there is nothing to {}. Run the sync again from the feature's sync banner.",
        step_id,
        match action {
            RunAction::Replay => "replay from",
            _ => "retry",
        }
    ))
}

/// Whether a step in `status` may be retried. `Some` is the refusal.
///
/// A retry rewinds the step and re-arms the driver, so the statuses it accepts
/// are exactly the ones with nothing in flight to race: a step that already
/// stopped (`failed`, `interrupted`) and one that never started (`pending`).
pub fn retry_refusal(status: &str) -> Option<String> {
    if status == "failed" || status == "interrupted" || status == "pending" {
        return None;
    }
    Some(format!(
        "Cannot retry a step in '{}' status. Only failed or interrupted steps can be retried.",
        status
    ))
}

/// Refuse to act on `target` while any of its graph *ancestors* is still
/// non-terminal (`pending`, `running`, `verifying`, or `awaiting_gate`), so a
/// stale retry / approve click does not race a still-running dependency.
///
/// `ancestors` is the resolved ancestor set — for a v1 chain exactly the old
/// `step_index <` predecessor set, and for a DAG only the upstream cone, which
/// correctly leaves independent branches undisturbed. `None` means the caller
/// could not resolve the graph (legacy feature without a matching workflow,
/// unparseable version row); the guard then falls back to the index
/// comparison rather than failing open — a guard that cannot see the graph
/// must still block *something*, and the index ordering is what every one of
/// those features ran under anyway.
///
/// `intent` is the user-facing phrase that follows "before" in the returned
/// message (e.g. "retrying this step", "deciding this gate"). It is purely
/// cosmetic so the call sites can give the user a tailored sentence.
pub fn active_predecessor_refusal(
    target: &StepExecution,
    siblings: &[StepExecution],
    ancestors: Option<&HashSet<StepId>>,
    intent: &str,
) -> Option<String> {
    for s in siblings {
        if s.id == target.id {
            continue;
        }
        let blocks = match ancestors {
            Some(set) => set.contains(&s.step_id),
            None => s.step_index < target.step_index,
        };
        if !blocks {
            continue;
        }
        if matches!(
            s.status.as_str(),
            "pending" | "running" | "verifying" | "awaiting_gate"
        ) {
            return Some(format!(
                "Step '{}' is still {}; wait for it to finish before {}.",
                s.step_id.0, s.status, intent
            ));
        }
    }
    None
}

#[cfg(test)]
#[path = "../../tests/domain/run_control.rs"]
mod tests;
