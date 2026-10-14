use super::cache_release::{retry_pending_runner_releases, RunnerCacheRpc};
use super::credentials::inject_pat_for_run;
use super::reconcile::{reconcile_one_run, NOTIFY_ON};
use super::rpc::{json_str, remote_rpc};
use crate::domain::ids::FeatureId;
use crate::domain::models::EffortLevel;
use crate::domain::shadow_settled::shadow_settled;
use crate::domain::step_assignment::StepAssignment;
use crate::error::AppError;
use crate::ports::db::FeatureRepository;
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::state::AppContext;

pub async fn cancel_remote_run(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<(), AppError> {
    let result = remote_rpc(
        ctx,
        &machine_id,
        "cancel_run",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)?;
    let status = json_str(&result, "status").unwrap_or_else(|| "cancelled".to_string());
    ctx.remote_run_mirror
        .update_status(
            &machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            crate::paths::now_ms(),
        )
        .map_err(AppError::from)?;
    Ok(())
}

/// Which rewind a detached run should perform — the wire value of the
/// runner's `retry_step` `mode` parameter.
///
/// The two are not interchangeable, and treating them as one is how remote
/// replay stayed broken: the runner's `Retry` arm refuses any step that is
/// not `failed` / `interrupted` / `pending`, and a replay target is
/// normally `completed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteRewind {
    /// Resume the node that broke, keeping a sequence step's landed prefix.
    Retry,
    /// An explicit redo from a node of any status, dropping that prefix.
    Replay,
}

impl RemoteRewind {
    fn as_wire(self) -> &'static str {
        match self {
            Self::Retry => "retry",
            Self::Replay => "replay",
        }
    }
}

/// The three run-shape overrides a rewind may re-pin, which always travel
/// together: the modal that starts one offers exactly these, each meaning
/// "leave it as it was" when `None`.
#[derive(Debug, Clone, Default)]
pub struct RewindOverrides {
    pub model: Option<String>,
    pub agent_kind: Option<String>,
    pub effort: Option<EffortLevel>,
}

pub async fn retry_remote_step(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    overrides: RewindOverrides,
    mode: RemoteRewind,
) -> Result<(), AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Err(AppError::not_found(format!(
            "No detached run {run_id} on machine {machine_id}"
        )));
    };
    inject_pat_for_run(ctx, &machine_id, &run_id, &row).await?;
    let result = remote_rpc(
        ctx,
        &machine_id,
        "retry_step",
        serde_json::json!({
            "run_id": run_id,
            "step_execution_id": step_execution_id,
            "model": overrides.model,
            "agent_kind": overrides.agent_kind,
            "effort": overrides.effort,
            "mode": mode.as_wire(),
        }),
    )
    .await
    .map_err(AppError::from)?;
    let status = json_str(&result, "status").unwrap_or_else(|| "running".to_string());
    ctx.remote_run_mirror
        .update_status(
            &machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            crate::paths::now_ms(),
        )
        .map_err(AppError::from)?;
    reconcile_one_run(ctx, &row).await;
    Ok(())
}

/// Pin one step's agent / model / effort on the runner that owns the run.
///
/// The twin of [`retry_remote_step`], deliberately down to the order of its
/// three stages: the mirror lookup is what proves the run is still ours to
/// address, `inject_pat_for_run` refreshes the credential the next spawn of
/// that node will need, and the single `reconcile_one_run` afterwards is how
/// the shadow row learns the pin — the runner is authoritative for a detached
/// run's `step_overrides`, so the desktop reads it back rather than writing a
/// local copy the next hydration would clobber.
///
/// What it does **not** borrow is the `update_status` call. A pin re-opens
/// nothing: the run keeps whatever status it had, so there is no transition to
/// mirror, and `retry_step`'s `unwrap_or("running")` default would report a
/// parked or awaiting-gate run as live.
///
/// A runner older than this method answers `unknown method:
/// set_step_assignment`. That arrives here as the RPC's `Err` and is returned
/// verbatim, because the pin did not land: nothing in this path may turn an
/// unapplied assignment into an `Ok`.
pub async fn set_remote_step_assignment(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    assignment: StepAssignment,
) -> Result<(), AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Err(AppError::not_found(format!(
            "No detached run {run_id} on machine {machine_id}"
        )));
    };
    inject_pat_for_run(ctx, &machine_id, &run_id, &row).await?;
    remote_rpc(
        ctx,
        &machine_id,
        "set_step_assignment",
        serde_json::json!({
            "run_id": run_id,
            "step_execution_id": step_execution_id,
            "agent_kind": assignment.agent_kind,
            "model": assignment.model,
            "effort": assignment.effort,
        }),
    )
    .await
    .map_err(AppError::from)?;
    reconcile_one_run(ctx, &row).await;
    Ok(())
}

pub async fn reinject_credentials(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    inject_pat_for_run(ctx, &machine_id, &run_id, &row).await?;
    reconcile_one_run(ctx, &row).await;
    ctx.remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)
}

pub async fn refresh_remote_run(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    reconcile_one_run(ctx, &row).await;
    ctx.remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)
}

/// What [`refresh_run_for_feature`] found when it went to the runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureRunRefresh {
    /// No mirror row names this feature: it runs on the project's own
    /// compute, so there is no runner to ask and nothing was changed.
    NotDetached,
    Refreshed,
    /// The mirror row was marked `unreachable` (or, for a run already
    /// terminal, left as it was) and the shadow is as of the last
    /// successful poll.
    RunnerUnreachable {
        machine_id: String,
    },
}

/// Pulls one feature's detached run from its runner through the same
/// [`reconcile_one_run`] the feature view's poll uses.
///
/// Keyed by feature because that is what an MCP client holds; the Tauri
/// command is keyed by `(machine_id, run_id)` because the UI holds the row.
/// `reconcile_one_run` returns `None` for both "runner unanswered" and "row
/// gone since the lookup", so the row is read again to tell them apart.
pub async fn refresh_run_for_feature(
    ctx: &AppContext,
    feature_id: &str,
) -> Result<FeatureRunRefresh, AppError> {
    let Some(row) = find_mirror_for_feature(ctx, feature_id.to_string())? else {
        return Ok(FeatureRunRefresh::NotDetached);
    };
    if reconcile_one_run(ctx, &row).await.is_some() {
        return Ok(FeatureRunRefresh::Refreshed);
    }
    match ctx
        .remote_run_mirror
        .get(&row.machine_id, &row.run_id)
        .map_err(AppError::from)?
    {
        Some(_) => Ok(FeatureRunRefresh::RunnerUnreachable {
            machine_id: row.machine_id,
        }),
        None => Ok(FeatureRunRefresh::NotDetached),
    }
}

pub fn list_mirrored_runs(ctx: &AppContext) -> Result<Vec<RemoteRunMirror>, AppError> {
    ctx.remote_run_mirror.list().map_err(AppError::from)
}

/// A mirror row as the feature view reads it: the row, plus whether its
/// shadow still has anything to learn from the runner.
///
/// The second is not derivable from the row — it needs the shadow feature —
/// and the view must not guess it from `status`, which is how a terminal row
/// over a feature still in flight stopped being polled.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MirroredRunView {
    #[serde(flatten)]
    pub run: RemoteRunMirror,
    pub shadow_settled: bool,
}

pub fn mirrored_run_view(
    features: &dyn FeatureRepository,
    run: RemoteRunMirror,
) -> MirroredRunView {
    let shadow_status = run
        .feature_id
        .as_ref()
        .and_then(|id| features.get(&FeatureId::from(id.clone())).ok().flatten())
        .map(|feature| feature.status);
    MirroredRunView {
        shadow_settled: shadow_settled(&run.status, shadow_status.as_deref()),
        run,
    }
}

pub fn find_mirror_for_feature(
    ctx: &AppContext,
    feature_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    Ok(ctx
        .remote_run_mirror
        .list()
        .map_err(AppError::from)?
        .into_iter()
        .find(|row| row.feature_id.as_deref() == Some(feature_id.as_str())))
}

pub async fn reconcile_all_runs(
    ctx: &AppContext,
    notify: &(dyn Fn(&[String]) + Sync),
) -> Result<Vec<RemoteRunMirror>, AppError> {
    let runner = RunnerCacheRpc {
        exec: ctx.exec.clone(),
        app_settings: ctx.app_settings.clone(),
    };
    if let Err(error) = retry_pending_runner_releases(&runner, &*ctx.app_settings).await {
        eprintln!("[RunnerCache] pending releases not retried: {error}");
    }
    let rows = ctx.remote_run_mirror.list().map_err(AppError::from)?;
    let mut notify_bodies = Vec::new();
    for row in &rows {
        let Some((status, error)) = reconcile_one_run(ctx, row).await else {
            continue;
        };
        if NOTIFY_ON.contains(&status.as_str())
            && row.last_notified_status.as_deref() != Some(status.as_str())
        {
            let body = match status.as_str() {
                "awaiting_mr" | "completed" => format!("{} — PR ready", row.title),
                "failed" => format!(
                    "{} — failed{}",
                    row.title,
                    error
                        .as_deref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                ),
                "parked" => format!("{} — parked, needs your decision", row.title),
                "over-budget" => format!("{} — hit its budget cap", row.title),
                "needs-credentials" => {
                    format!("{} — needs credentials re-injected", row.title)
                }
                _ => row.title.clone(),
            };
            notify_bodies.push(body);
            let _ = ctx
                .remote_run_mirror
                .mark_notified(&row.machine_id, &row.run_id, &status);
        }
    }
    if !notify_bodies.is_empty() {
        notify(&notify_bodies);
    }
    ctx.remote_run_mirror.list().map_err(AppError::from)
}
