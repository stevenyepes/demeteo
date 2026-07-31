//! Breaking the redirect loop: the durable writes a `redirect` decision owes.

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::StepExecution;
use crate::ports::db::{FeatureRepository, GateRepository, StepExecutionPatch};
use crate::ports::notification::{DomainEvent, NotificationPort};

/// The three repositories a redirect writes through. They are never
/// addressable apart: every write below is paired with the event that pushes
/// it, and the gate row and the target row are reset in one breath.
pub(super) struct GateWriters<'a> {
    pub features: &'a dyn FeatureRepository,
    pub gates: &'a dyn GateRepository,
    pub notif: &'a dyn NotificationPort,
}

/// Which row the redirect rewinds, and which gate sent it there.
pub(super) struct RedirectReset<'a> {
    /// All step executions for the current run, in order.
    pub step_execs: &'a [StepExecution],
    pub target_idx: usize,
    pub gate_step_execution_id: &'a StepExecutionId,
}

/// Apply the durable state changes that a `redirect` gate decision
/// requires. Pulled out of [`ExecutionDriver::apply_gate_decision`]
/// so the loop-breaking fix is unit-testable without a full
/// `ExecutionDriver` (and so the in-line `apply_gate_decision`
/// branch stays a short redirect that delegates the work here).
///
/// Concretely:
///   * the target step is reset to status `pending` (with all
///     counters cleared and artifacts dropped) so the driver's
///     resume-skip logic does not treat it as already-completed and
///     skip past it;
///   * the gate's own status row is flipped from `awaiting_gate`
///     back to `pending` so the timeline stops displaying the
///     "Decide Gate" affordance while the redirected step is
///     re-running (the gate will re-emit `awaiting_gate` on its
///     next visit); and
///   * the gate's own `gate_decisions` row is cleared so the next
///     visit to the gate re-prompts the user. Without this third
///     half, the gate's reconciliation would find the prior
///     `redirect` decision on file, return
///     `RedirectTo(target_idx)` once more, and the same step would
///     loop forever — the bug this fix exists to break.
///
/// Each DB mutation is paired with a `StepProgress` event so the
/// frontend's local `steps` array picks up the new status without
/// waiting for a full `step_list_for_run` poll. Missing the event
/// leaves the timeline showing "Decide Gate" / "Retry Step" for
/// rows whose DB state has already moved on (the bug this fix
/// exists to break in the UI layer).
///
/// All writes are best-effort. Failures are intentionally
/// swallowed: the redirect already won the user's intent, and any
/// stale state is recoverable on the next reconciliation pass
/// (the startup watchdog will re-surface the gate if the driver
/// dies between the reset and the target step completing).
///
/// [`ExecutionDriver::apply_gate_decision`]: crate::adapters::step_executor::driver::ExecutionDriver
pub(super) fn reset_gate_target(
    writers: GateWriters<'_>,
    f_id: &FeatureId,
    reset: RedirectReset<'_>,
) {
    let GateWriters {
        features,
        gates,
        notif,
    } = writers;
    let RedirectReset {
        step_execs,
        target_idx,
        gate_step_execution_id,
    } = reset;

    if let Some(target_exec) = step_execs.get(target_idx) {
        // Reset every counter / artifact the previous attempt
        // accumulated so the re-run starts from a clean slate.
        // `cost_usd` / `tokens` / `wall_clock_secs` are wrapped in
        // `Some(Some(0))` because the patch type uses
        // `Option<Option<T>>` to distinguish "leave alone" (`None`)
        // from "set to value" (`Some(Some(v))`).
        let _ = features.step_update(
            &target_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("pending".to_string()),
                cost_usd: Some(Some(0.0)),
                tokens: Some(Some(0)),
                wall_clock_secs: Some(Some(0)),
                artifact_path: Some(None),
                artifact_paths: Some(Vec::new()),
                error_message: Some(None),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = notif.emit(&DomainEvent::StepProgress {
            feature_id: f_id.clone(),
            step_id: target_exec.step_id.0.clone(),
            status: "pending".into(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
    }
    // Clear this gate's own decision row so the next visit to the
    // gate re-prompts the user. Idempotent against app restarts: if
    // the driver dies after the reset and before the target step
    // finishes, the startup watchdog will already mark the gate
    // `interrupted` and create a fresh `gate_decisions` row with
    // `decision = None` (see `startup_watchdog` in
    // `impl_traits/startup_recovery.rs`).
    let _ = gates.reset_for_step_execution(gate_step_execution_id);
    // Flip the gate's own status from `awaiting_gate` to `pending`
    // so the timeline stops showing the "Decide Gate" button while
    // the redirected step re-runs. Without this update the gate
    // remains `awaiting_gate` in the DB and the frontend's stale
    // local cache keeps rendering the decision affordance — even
    // though the user already submitted a decision and the gate
    // won't re-prompt until the target finishes. Fetch the row
    // first so we have the gate's `step_id` to put in the event.
    if let Ok(Some(gate_exec)) = features.step_get(gate_step_execution_id) {
        let _ = features.step_update(
            gate_step_execution_id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("pending".to_string()),
                cost_usd: None,
                tokens: None,
                wall_clock_secs: None,
                artifact_path: None,
                artifact_paths: None,
                error_message: None,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = notif.emit(&DomainEvent::StepProgress {
            feature_id: f_id.clone(),
            step_id: gate_exec.step_id.0.clone(),
            status: "pending".into(),
            cost_usd: None,
            tokens: None,
            wall_clock_secs: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
    }
}

/// The bug this regression suite exists to break: when a gate
/// redirects back to a previous step with feedback, the orchestrator
/// used to re-run the target step, then re-enter the gate, find the
/// same `redirect` decision on file, redirect back again — and loop
/// forever. `reset_gate_target` is the fix: it resets the target
/// step's status to `pending` and clears the gate's own decision
/// row. These tests pin both halves of the fix in place.
#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/gate_redirect_reset.rs"]
mod redirect_reset_tests;
