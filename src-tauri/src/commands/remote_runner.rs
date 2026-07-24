#![allow(clippy::useless_conversion)]

use crate::error::AppError;
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::state::AppContext;
pub use demeteo_core::application::remote_runs::RemoteRunHandle;
use demeteo_core::application::remote_runs::*;
use tauri::{AppHandle, State};
use tauri_plugin_notification::NotificationExt;

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn remote_submit_run(
    ctx: State<'_, AppContext>,
    machine_id: String,
    project_id: String,
    workflow_id: String,
    title: String,
    description: String,
    agent_kind: Option<String>,
    model: Option<String>,
    effort: Option<crate::domain::models::EffortLevel>,
    commit_artifacts: Option<bool>,
    loop_iterations: Option<u32>,
    max_budget_usd: Option<f64>,
    step_overrides: Option<Vec<crate::domain::models::StepOverride>>,
    staged_attachments: Option<Vec<crate::commands::attachments::StagedAttachmentInput>>,
    target_repo_id: Option<String>,
    unattended: bool,
    max_cost_usd: Option<f64>,
    max_wall_clock_secs: Option<u64>,
) -> Result<RemoteRunHandle, AppError> {
    submit_remote_run(
        &ctx,
        SubmitInput {
            machine_id,
            project_id,
            workflow_id,
            title,
            description,
            agent_kind,
            model,
            effort,
            commit_artifacts,
            loop_iterations,
            max_budget_usd,
            step_overrides,
            staged_attachments,
            target_repo_id,
            unattended,
            max_cost_usd,
            max_wall_clock_secs,
        },
    )
    .await
    .map(RemoteRunHandle::from)
    .map_err(AppError::from)
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
) -> Result<Option<String>, AppError> {
    resolve_run_diff_url(&ctx, project_id, branch).map_err(AppError::from)
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
        model,
        agent_kind,
        effort,
    )
    .await
    .map_err(AppError::from)
}
