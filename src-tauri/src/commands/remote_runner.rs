#![allow(clippy::useless_conversion)]

use crate::application::launch::{launch_run as launch, LaunchRequest, LaunchedRun};
use crate::domain::run_placement::{placement_from_raw, DetachedOptions};
use crate::error::AppError;
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;
use demeteo_core::application::remote_runs::*;
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

/// The frontend's `launchRun` input (`src/lib/launch.ts`), taken as one
/// camelCase `args` object (`{ args: { machineId, … } }`) that
/// `useLaunchRun.test.tsx` pins — change all three together. An absent,
/// blank or `"local"` `machine_id` launches on the project's own compute; any
/// other id is a detached run on that machine — see
/// [`crate::domain::run_placement`]. The app version a detached run is checked
/// against is not part of it: core reads it from [`AppContext::app_version`].
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRunArgs {
    pub machine_id: Option<String>,
    pub project_id: String,
    pub workflow_id: String,
    pub title: String,
    pub description: String,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<crate::domain::models::EffortLevel>,
    pub commit_artifacts: Option<bool>,
    pub loop_iterations: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub step_overrides: Option<Vec<crate::domain::models::StepOverride>>,
    pub staged_attachments: Option<Vec<crate::commands::attachments::StagedAttachmentInput>>,
    pub target_repo_id: Option<String>,
    pub unattended: Option<bool>,
    pub max_cost_usd: Option<f64>,
    pub max_wall_clock_secs: Option<u64>,
    pub origin: Option<crate::domain::feature_origin::FeatureOrigin>,
    pub diff_base_branch: Option<String>,
}

impl LaunchRunArgs {
    fn into_launch_request(self) -> LaunchRequest {
        LaunchRequest {
            placement: placement_from_raw(self.machine_id.as_deref()),
            launch: FeatureLaunch {
                project_id: self.project_id,
                workflow_id: self.workflow_id,
                title: self.title,
                description: self.description,
                agent_kind: self.agent_kind,
                model: self.model,
                effort: self.effort,
                commit_artifacts: self.commit_artifacts,
                loop_iterations: self.loop_iterations,
                max_budget_usd: self.max_budget_usd,
                step_overrides: self.step_overrides.unwrap_or_default(),
                staged_attachments: self.staged_attachments.unwrap_or_default(),
                origin: self.origin.unwrap_or_default(),
                diff_base_branch: self.diff_base_branch,
                ..FeatureLaunch::default()
            },
            detached: DetachedOptions {
                target_repo_id: self.target_repo_id,
                unattended: self.unattended,
                max_cost_usd: self.max_cost_usd,
                max_wall_clock_secs: self.max_wall_clock_secs,
            },
        }
    }
}

#[tauri::command]
pub async fn launch_run(
    ctx: State<'_, AppContext>,
    args: LaunchRunArgs,
) -> Result<LaunchedRun, AppError> {
    launch(&ctx, args.into_launch_request()).await
}

#[tauri::command]
pub async fn remote_reinject_credentials(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    reinject_credentials(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn remote_list_mirrored_runs(
    ctx: State<'_, AppContext>,
) -> Result<Vec<RemoteRunMirror>, AppError> {
    list_mirrored_runs(&ctx).map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_reconcile_runs(
    app: AppHandle,
    ctx: State<'_, AppContext>,
) -> Result<Vec<RemoteRunMirror>, AppError> {
    let notify = |notify_bodies: &[String]| {
        let (title, body) = if notify_bodies.len() == 1 {
            ("Demeteo — remote run".to_string(), notify_bodies[0].clone())
        } else {
            (
                format!(
                    "Demeteo — {} remote runs need attention",
                    notify_bodies.len()
                ),
                notify_bodies.join("\n"),
            )
        };
        let _ = app.notification().builder().title(title).body(body).show();
    };
    reconcile_all_runs(&ctx, &notify)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_refresh_run(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    refresh_remote_run(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn remote_run_for_feature(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    find_mirror_for_feature(&ctx, feature_id).map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_get_status(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    get_status(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn remote_run_diff_url(
    ctx: State<'_, AppContext>,
    project_id: String,
    branch: String,
    feature_id: Option<String>,
) -> Result<Option<String>, AppError> {
    resolve_run_diff_url(&ctx, project_id, branch, feature_id).map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_stream_events(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    from_offset: i64,
) -> Result<serde_json::Value, AppError> {
    stream_events(&ctx, machine_id, run_id, from_offset)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_get_feature(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    get_feature(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_list_steps(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    list_steps(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_read_artifact(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    path: String,
) -> Result<serde_json::Value, AppError> {
    read_artifact(&ctx, machine_id, run_id, path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_list_messages(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    thread_id: String,
) -> Result<serde_json::Value, AppError> {
    list_messages(&ctx, machine_id, run_id, thread_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_get_worktree(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    get_worktree(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_decide_gate(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    gate_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), AppError> {
    decide_gate(&ctx, machine_id, run_id, gate_id, decision, feedback)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_cancel_run(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
) -> Result<(), AppError> {
    cancel_remote_run(&ctx, machine_id, run_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn remote_retry_step(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    model: Option<String>,
    agent_kind: Option<String>,
    effort: Option<crate::domain::models::EffortLevel>,
) -> Result<(), AppError> {
    retry_remote_step(
        &ctx,
        machine_id,
        run_id,
        step_execution_id,
        RewindOverrides {
            model,
            agent_kind,
            effort,
        },
        RemoteRewind::Retry,
    )
    .await
    .map_err(AppError::from)
}

/// Pin one queued step's agent, model and effort on a detached run — the
/// remote twin of `step_set_assignment`, carrying the same meaning: tier 1
/// only, no rewind, and **all three fields `None` is the "reset to inherited"
/// request**, which removes that step's pin. It is never a no-op patch that
/// leaves the existing pin standing.
///
/// A runner predating the `set_step_assignment` RPC answers `unknown method`,
/// which surfaces here as an `Err` rather than a silent success.
#[tauri::command]
pub async fn remote_set_step_assignment(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    agent_kind: Option<String>,
    model: Option<String>,
    effort: Option<crate::domain::models::EffortLevel>,
) -> Result<(), AppError> {
    set_remote_step_assignment(
        &ctx,
        machine_id,
        run_id,
        step_execution_id,
        crate::domain::step_assignment::StepAssignment {
            agent_kind,
            model,
            effort,
        },
    )
    .await
    .map_err(AppError::from)
}

/// Replay a detached run from a step — the remote twin of
/// `replay_from_step`, and deliberately *not* `remote_retry_step` with a
/// different label. The runner's retry arm rejects any step that is not
/// `failed` / `interrupted` / `pending`, and a replay target is normally a
/// `completed` one; it also keeps a sequence step's landed prefix, which an
/// explicit redo must drop.
#[tauri::command]
pub async fn remote_replay_step(
    ctx: State<'_, AppContext>,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    model: Option<String>,
    agent_kind: Option<String>,
    effort: Option<crate::domain::models::EffortLevel>,
) -> Result<(), AppError> {
    retry_remote_step(
        &ctx,
        machine_id,
        run_id,
        step_execution_id,
        RewindOverrides {
            model,
            agent_kind,
            effort,
        },
        RemoteRewind::Replay,
    )
    .await
    .map_err(AppError::from)
}

#[cfg(test)]
#[path = "../../tests/infrastructure/remote_runner.rs"]
mod tests;
