//! What stopping or rewinding a run means on this runner, whoever asked: the
//! control socket's `cancel_run` and `retry_step`, and a Hub cancel or retry
//! (`hub::runs`), all end here, so a run the Hub stopped or retried and one a
//! desktop did leave the same row and the same events (decision 64, read per
//! instance).
//!
//! None checks an owner. The control socket checks one before calling in;
//! the Hub acts for the whole runner (docs/HUB.md).

use crate::credentials::CredentialStore;
use crate::services::RunnerServices;
use demeteo_core::domain::models::EffortLevel;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::error::AppError;
use demeteo_core::paths;
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};
use demeteo_core::state::AppContext;
use std::sync::Arc;

/// Which step [`rewind_run`] rewinds to and how: the control socket's
/// `retry_step` params, its `mode` read as `replay`. A Hub retry is a
/// `Retry` with no overrides, as the desktop's own retry button is.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rewind<'a> {
    pub step_execution_id: &'a str,
    pub replay: bool,
    pub model: Option<&'a str>,
    pub agent_kind: Option<&'a str>,
    pub effort: Option<EffortLevel>,
}

/// The executor's rewind, then the run re-opened.
///
/// The second half is the subtle one. The step's failure already drove the
/// feature terminal, so this run's `await_terminal_and_push` tail has
/// already exited and stamped the run row `failed`. `step_retry` re-arms
/// the engine's driver, and the engine will happily run the pipeline to
/// completion — but with nobody awaiting it, the terminal push + PR-open
/// (which live *after* that poll loop) would never happen, and the laptop,
/// seeing a terminal run row, would have stopped polling for progress. So
/// re-open the run: flip it back to `running` and start a driver over the
/// persisted spec, exactly as the restart path in `reconcile.rs` does. When a
/// driver for the run is already live this attaches to it instead and puts the
/// row back as it was; the live driver runs one more pass and settles it.
///
/// A step the executor declines comes back `AppError::Validation`, which a
/// Hub retry answers `step_not_retryable`.
pub(crate) async fn rewind_run(
    svc: &Arc<RunnerServices>,
    run: &RunnerRun,
    rewind: Rewind<'_>,
) -> Result<(), AppError> {
    let spec: RunSpec = serde_json::from_str(&run.spec_json).map_err(|e| {
        AppError::internal(format!("run {} has an unparseable spec: {}", run.run_id, e))
    })?;
    if run.project_id.is_none() || run.feature_id.is_none() {
        return Err(AppError::validation(format!(
            "run {} never bootstrapped a feature; there is no step to retry",
            run.run_id
        )));
    };

    let executor = &svc.ctx.executor;
    if rewind.replay {
        executor
            .replay_from_step(
                rewind.step_execution_id,
                rewind.model,
                rewind.agent_kind,
                rewind.effort,
            )
            .await
            .map_err(AppError::internal)?;
    } else {
        executor
            .step_retry(
                rewind.step_execution_id,
                rewind.model,
                rewind.agent_kind,
                rewind.effort,
            )
            .await?;
    }

    let event = if rewind.replay { "replayed" } else { "retried" };
    crate::run::emit(&svc.ctx, &run.run_id, event, rewind.step_execution_id);
    crate::run::reopen_run_driver(svc, &run.run_id, spec).map_err(AppError::internal)?;
    Ok(())
}

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
