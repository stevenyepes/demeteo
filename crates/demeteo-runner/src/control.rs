//! What stopping a run means on this runner, whoever asked: the control
//! socket's `cancel_run` and a Hub cancel (`hub::runs`) both end here, so a
//! run the Hub stopped and one a desktop stopped leave the same row, the same
//! events and no credential (decision 64, read per instance).
//!
//! Neither checks an owner. The control socket checks one before calling in;
//! the Hub acts for the whole runner (docs/HUB.md).

use crate::credentials::CredentialStore;
use demeteo_core::paths;
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};
use demeteo_core::state::AppContext;

/// R8: cancellation is explicit — closing the laptop or dropping the SSH
/// tunnel must never cancel a run. The feature is stopped by the same
/// `StepExecutor::feature_cancel` the desktop app's stop button uses.
pub(crate) async fn cancel_run(
    ctx: &AppContext,
    creds: &CredentialStore,
    run: RunnerRun,
) -> Result<RunnerRun, String> {
    if let Some(feature_id) = &run.feature_id {
        ctx.executor
            .feature_cancel(feature_id)
            .await
            .map_err(|e| format!("failed to cancel feature: {}", e))?;
    }

    // Atomic conditional update (not read-then-write): if the run
    // finished for real between the caller's `get` and this call — e.g. the
    // background execute_run task raced us to a genuine `awaiting_mr` —
    // this leaves that real outcome alone instead of stomping it to
    // `cancelled`. Returns whatever the row's true status ends up being.
    let now = paths::now_ms();
    let run = ctx
        .runner_runs
        .cancel_if_active(&run.run_id, now)?
        .ok_or_else(|| "run vanished during cancel".to_string())?;
    if run.status == "cancelled" {
        ctx.run_events.append(&run.run_id, "cancelled", None, now)?;
        // §6.2: wiped at run end — success, failure, or cancel.
        creds.remove(&run.run_id);
    }
    Ok(run)
}

/// The run that executes `feature_id`, newest first if a feature somehow has
/// more than one. The Hub names a run by the Feature id it was reported
/// under, never by run id.
pub(crate) fn run_for_feature(
    runs: &dyn RunnerRunPort,
    feature_id: &str,
) -> Result<Option<RunnerRun>, String> {
    Ok(runs
        .list()?
        .into_iter()
        .find(|run| run.feature_id.as_deref() == Some(feature_id)))
}

#[cfg(test)]
#[path = "control_tests.rs"]
mod tests;
