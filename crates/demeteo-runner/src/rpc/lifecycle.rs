use crate::services::RunnerServices;
use demeteo_core::domain::ids::{FeatureId, ProjectId};
use demeteo_core::domain::models::EffortLevel;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::domain::step_assignment::StepAssignment;
use demeteo_core::paths;
use demeteo_core::ports::db::FeatureRepository;
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};
use serde::Deserialize;
use std::sync::Arc;

use super::ownership::{no_such_step, require_owner, require_owner_of_step};
use super::RunIdParams;

#[derive(Debug, Deserialize)]
struct SubmitRunParams {
    run_id: String,
    spec: RunSpec,
}

/// Which rewind `retry_step` performs — the remote twin of the desktop's
/// two distinct commands.
///
/// They are *not* the same operation, and routing both through
/// `step_retry` (as the laptop did before this field existed) makes replay
/// impossible on a detached run: `step_retry` only accepts a step in
/// `failed` / `interrupted` / `pending`, and the step a human replays from
/// is almost always `completed`. It also keeps any landed sequence prefix,
/// so an explicit redo would skip most of its own task list.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum RetryMode {
    /// Resume the node that broke, keeping a sequence step's landed
    /// prefix. The default, so a desktop older than this runner — which
    /// omits the field entirely — keeps its existing behaviour.
    #[default]
    Retry,
    /// An explicit redo from a node of any status, dropping the landed
    /// prefix so the node runs its whole list again.
    Replay,
}

/// `retry_step(run_id, step_execution_id, model?, agent_kind?, effort?,
/// mode?)` — the remote twin of the desktop app's `step_retry` *and*
/// `replay_from_step` commands, selected by `mode`. Unlike `decide_gate`,
/// `run_id` is load-bearing here: either rewind has to re-open the run
/// (the step's failure already drove it terminal, so its
/// `await_terminal_and_push` tail has exited), and that is keyed by run.
///
/// `effort` is `#[serde(default)]` like the rest: a desktop app older than
/// this runner simply omits it, and the retry keeps the feature's existing
/// effort override.
#[derive(Debug, Deserialize)]
struct RetryStepParams {
    run_id: String,
    step_execution_id: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    agent_kind: Option<String>,
    #[serde(default)]
    effort: Option<EffortLevel>,
    #[serde(default)]
    mode: RetryMode,
}

/// `set_step_assignment(run_id, step_execution_id, agent_kind?, model?,
/// effort?)`.
///
/// Every dimension is optional-by-default on the wire, and an omitted one
/// means *unpinned*, not *unchanged*: the trio submitted here becomes the
/// stored pin wholesale. There is deliberately no wire spelling for "leave
/// this dimension as it was" — see
/// [`apply_step_assignment`](demeteo_core::domain::step_assignment::apply_step_assignment)
/// for why the surface sends all three.
#[derive(Debug, Deserialize)]
struct SetStepAssignmentParams {
    run_id: String,
    step_execution_id: String,
    #[serde(default)]
    agent_kind: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    effort: Option<EffortLevel>,
}

/// Idempotent by `run_id` (R9/M3.2): re-submitting the same `run_id`
/// returns the existing row instead of starting a second feature. A
/// freshly-created row is handed off to `crate::run::execute_run` on a
/// spawned task and this returns immediately — `submit_run` reports
/// "accepted", not "finished"; the caller polls `get_status`/`list_runs`
/// (or, from M3.3 on, tails the event log).
pub(super) async fn submit_run(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<RunnerRun, String> {
    let params: SubmitRunParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let spec_json = serde_json::to_string(&params.spec).map_err(|e| e.to_string())?;
    let now = paths::now_ms();
    // MC-D2: stamp the owning client at creation. `get_or_create` sets it
    // only on the insert, so a re-submit never re-homes an existing run.
    let run = svc
        .ctx
        .runner_runs
        .get_or_create(&params.run_id, &spec_json, client_id, now)?;

    if run.status != "pending" {
        // Already submitted (possibly already finished) — no-op.
        return Ok(run);
    }

    // M4.1: agent-readiness precondition. Fail loud at launch rather than
    // mid-run — a machine missing the selected agent binary is ineligible
    // for this run entirely.
    if let Some(kind) = params.spec.agent_kind.as_deref() {
        if !svc
            .ctx
            .registry
            .is_available(kind, svc.ctx.exec.as_ref(), "local", false)
            .await
        {
            let msg = format!(
                "agent '{}' is not installed/available on this machine — run rejected",
                kind
            );
            svc.ctx.runner_runs.update_status(
                &params.run_id,
                "failed",
                None,
                None,
                Some(&msg),
                None,
                now,
            )?;
            return svc
                .ctx
                .runner_runs
                .get(&params.run_id)?
                .ok_or_else(|| "run vanished immediately after creation".to_string());
        }
    }

    svc.ctx
        .runner_runs
        .update_status(&params.run_id, "running", None, None, None, None, now)?;

    let svc_bg = svc.clone();
    let run_id_bg = params.run_id.clone();
    tokio::spawn(async move {
        let result = crate::run::execute_run(&svc_bg, &run_id_bg, &params.spec).await;
        let now = paths::now_ms();
        match result {
            Ok(outcome) => {
                let _ = svc_bg.ctx.runner_runs.update_status(
                    &run_id_bg,
                    &outcome.status,
                    outcome.project_id.as_deref(),
                    outcome.feature_id.as_deref(),
                    None,
                    outcome.pushed_branch.as_deref(),
                    now,
                );
            }
            Err(e) => {
                let _ = svc_bg.ctx.run_events.append(
                    &run_id_bg,
                    "failed",
                    serde_json::to_string(&e).ok().as_deref(),
                    now,
                );
                let _ = svc_bg.ctx.runner_runs.update_status(
                    &run_id_bg,
                    "failed",
                    None,
                    None,
                    Some(&e),
                    None,
                    now,
                );
            }
        }
    });

    svc.ctx
        .runner_runs
        .get(&params.run_id)?
        .ok_or_else(|| "run vanished immediately after creation".to_string())
}

/// Rewind a detached run to one of its steps from the laptop — the remote
/// twin of the desktop app's `step_retry` / `replay_from_step` commands,
/// which can only ever drive runs *this* machine owns (the local executor
/// refuses a runner-owned shadow outright). `mode` picks which, and the
/// distinction is not cosmetic — see [`RetryMode`].
///
/// The subtlety is the second half. The step's failure already drove the
/// feature terminal, so this run's `await_terminal_and_push` tail has
/// already exited and stamped the run row `failed`. `step_retry` re-arms
/// the engine's driver, and the engine will happily run the pipeline to
/// completion — but with nobody awaiting it, the terminal push + PR-open
/// (which live *after* that poll loop) would never happen, and the laptop,
/// seeing a terminal run row, would have stopped polling for progress. So
/// re-open the run: flip it back to `running` and spawn a fresh
/// `await_terminal_and_push` over the persisted spec, exactly as the
/// restart path in `reconcile.rs` does.
pub(super) async fn retry_step(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<RunnerRun, String> {
    let params: RetryStepParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let run = require_owner_of_step(
        svc.ctx.features.as_ref(),
        svc.ctx.runner_runs.as_ref(),
        &params.step_execution_id,
        client_id,
    )?;
    // The step id resolved to a run this client owns — but not necessarily
    // the run it *said* it was retrying. Reject the mismatch rather than
    // silently re-opening a different run than the caller named.
    if run.run_id != params.run_id {
        return Err(no_such_step(&params.step_execution_id));
    }

    let spec: RunSpec = serde_json::from_str(&run.spec_json)
        .map_err(|e| format!("run {} has an unparseable spec: {}", run.run_id, e))?;
    let (Some(project_id), Some(feature_id)) = (run.project_id.clone(), run.feature_id.clone())
    else {
        return Err(format!(
            "run {} never bootstrapped a feature; there is no step to retry",
            run.run_id
        ));
    };

    match params.mode {
        RetryMode::Retry => svc
            .ctx
            .executor
            .step_retry(
                &params.step_execution_id,
                params.model.as_deref(),
                params.agent_kind.as_deref(),
                params.effort,
            )
            .await
            .map_err(|e| e.to_string())?,
        RetryMode::Replay => {
            svc.ctx
                .executor
                .replay_from_step(
                    &params.step_execution_id,
                    params.model.as_deref(),
                    params.agent_kind.as_deref(),
                    params.effort,
                )
                .await?
        }
    }

    let now = paths::now_ms();
    svc.ctx
        .runner_runs
        .update_status(&run.run_id, "running", None, None, None, None, now)?;
    let event = match params.mode {
        RetryMode::Retry => "retried",
        RetryMode::Replay => "replayed",
    };
    crate::run::emit(&svc.ctx, &run.run_id, event, &params.step_execution_id);

    let svc_bg = svc.clone();
    let run_id_bg = run.run_id.clone();
    tokio::spawn(async move {
        let result = crate::run::await_terminal_and_push(
            &svc_bg,
            &run_id_bg,
            &ProjectId::from(project_id),
            &FeatureId::from(feature_id),
            &spec,
        )
        .await;
        let now = paths::now_ms();
        match result {
            Ok(outcome) => {
                let _ = svc_bg.ctx.runner_runs.update_status(
                    &run_id_bg,
                    &outcome.status,
                    outcome.project_id.as_deref(),
                    outcome.feature_id.as_deref(),
                    None,
                    outcome.pushed_branch.as_deref(),
                    now,
                );
            }
            Err(e) => {
                let _ = svc_bg.ctx.run_events.append(
                    &run_id_bg,
                    "failed",
                    serde_json::to_string(&e).ok().as_deref(),
                    now,
                );
                let _ = svc_bg.ctx.runner_runs.update_status(
                    &run_id_bg,
                    "failed",
                    None,
                    None,
                    Some(&e),
                    None,
                    now,
                );
            }
        }
    });

    svc.ctx
        .runner_runs
        .get(&run.run_id)?
        .ok_or_else(|| "run vanished during retry".to_string())
}

/// Pin which agent, model and effort one step of a detached run uses — the
/// remote twin of the desktop app's per-step Assignment control, and the
/// only way to change the assignment on a runner-owned run (the local
/// executor refuses a shadow row outright, so a laptop-side write would be
/// clobbered by the next hydration).
///
/// All three dimensions absent is the documented **reset to inherited**
/// request, not a no-op: the step's pin is removed and the node goes back to
/// resolving through the feature-wide tier.
///
/// Unlike [`retry_step`] this performs no rewind and re-opens nothing — the
/// run is left exactly as it was, and the pin applies the next time the
/// scheduler dispatches that node. So there is no status to flip back to
/// `running` and no `await_terminal_and_push` tail to respawn here.
pub(super) async fn set_step_assignment(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<(), String> {
    let params: SetStepAssignmentParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    assignment_target(
        svc.ctx.features.as_ref(),
        svc.ctx.runner_runs.as_ref(),
        &params,
        client_id,
    )?;

    svc.ctx
        .executor
        .step_set_assignment(&params.step_execution_id, assignment_of(&params))
        .await
        .map_err(|e| e.to_string())
}

/// The change [`set_step_assignment`] hands the executor: the wire's trio,
/// carried across verbatim. Nothing is merged over what the step was pinned
/// to before and nothing is filled in from a default — an omitted dimension
/// is an un-pinned one, so an empty payload has to arrive at the executor as
/// the reset request rather than being dropped on the way.
fn assignment_of(params: &SetStepAssignmentParams) -> StepAssignment {
    StepAssignment {
        agent_kind: params.agent_kind.clone(),
        model: params.model.clone(),
        effort: params.effort,
    }
}

/// [`set_step_assignment`]'s whole pre-executor gate: resolve the step to a
/// run this client owns, then confirm it is the run the caller named. A bare
/// `step_execution_id` is otherwise a bearer capability — any tunnelled
/// caller who learned one could re-point another client's step at an agent
/// of their choosing (MC-D2).
///
/// Free over the two ports it reads so each refusal is reachable from a test
/// without a [`RunnerServices`]; the byte-identity the two refusals owe each
/// other is [`no_such_step`]'s.
fn assignment_target(
    features: &dyn FeatureRepository,
    runs: &dyn RunnerRunPort,
    params: &SetStepAssignmentParams,
    client_id: &str,
) -> Result<(), String> {
    let run = require_owner_of_step(features, runs, &params.step_execution_id, client_id)?;
    if run.run_id != params.run_id {
        return Err(no_such_step(&params.step_execution_id));
    }
    Ok(())
}

/// R8: cancellation is explicit and RPC-only — closing the laptop or
/// dropping the SSH tunnel must never cancel a run. Delegates to the
/// same `StepExecutor::feature_cancel` the desktop app's `feature_cancel`
/// Tauri command already uses; no separate cancellation logic to drift.
pub(super) async fn cancel_run(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<RunnerRun, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    // MC-D2: only the owning client may cancel; a non-owner gets the
    // uniform "no such run" and the run keeps executing untouched.
    let run = require_owner(svc, &params.run_id, client_id)?;

    if let Some(feature_id) = &run.feature_id {
        svc.ctx
            .executor
            .feature_cancel(feature_id)
            .await
            .map_err(|e| format!("failed to cancel feature: {}", e))?;
    }

    // Atomic conditional update (not read-then-write): if the run
    // finished for real between our `get` above and this call — e.g. the
    // background execute_run task raced us to a genuine `awaiting_mr` —
    // this leaves that real outcome alone instead of stomping it to
    // `cancelled`. Returns whatever the row's true status ends up being.
    let now = paths::now_ms();
    let run = svc
        .ctx
        .runner_runs
        .cancel_if_active(&params.run_id, now)?
        .ok_or_else(|| "run vanished during cancel".to_string())?;
    if run.status == "cancelled" {
        svc.ctx
            .run_events
            .append(&params.run_id, "cancelled", None, now)?;
        // §6.2: wiped at run end — success, failure, or cancel.
        svc.creds.remove(&params.run_id);
    }
    Ok(run)
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
