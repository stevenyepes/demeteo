use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::error::AppError;
use crate::ports::step_executor::SyncOutcomeView;
use crate::state::AppContext;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct FeatureWorktreeInfo {
    pub machine_id: String,
    pub worktree_path: String,
    pub branch: String,
    pub default_branch: String,
}

#[tauri::command]
pub async fn feature_get_worktree(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<FeatureWorktreeInfo, AppError> {
    let fid = FeatureId::from(feature_id);
    let feature = ctx
        .features
        .get(&fid)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::from("Feature not found"))?;

    let project_id = feature.project_id;
    let all = ctx.projects.get_projects().map_err(AppError::from)?;
    let project = all
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| AppError::from("Project not found"))?;

    let repos = ctx
        .projects
        .get_repositories_for(&project_id)
        .map_err(AppError::from)?;
    let repo = repos
        .first()
        .ok_or_else(|| AppError::from("No repository configured for this project"))?;

    let settings = ctx
        .projects
        .get_settings(&project_id)
        .map_err(AppError::from)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);

    let machine_id = if project.compute_type.eq_ignore_ascii_case("local") {
        "local".to_string()
    } else {
        project
            .remote_host
            .as_ref()
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "local".to_string())
    };

    let worktree_path = crate::paths::repo_target_dir_str(
        &ctx.exec,
        &project.compute_type,
        project.remote_host.as_ref().map(|m| m.as_str()),
        &project_id.0,
        &repo.repo_path,
    )
    .await
    .map_err(AppError::from)?;

    let branch = format!(
        "{}{}",
        settings.worktree_strategy.branch_prefix,
        fid.0
    );

    Ok(FeatureWorktreeInfo {
        machine_id,
        worktree_path,
        branch,
        default_branch: settings.worktree_strategy.default_branch.clone(),
    })
}

#[tauri::command]
pub fn fetch_active_features(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<Feature>, AppError> {
    ctx.features
        .get_active(&crate::domain::ids::ProjectId::from(project_id))
        .map_err(AppError::from)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_feature(
    ctx: State<'_, AppContext>,
    project_id: String,
    workflow_id: String,
    title: String,
    description: String,
    agent_kind: Option<String>,
    model: Option<String>,
    commit_artifacts: Option<bool>,
) -> Result<Feature, AppError> {
    ctx.executor
        .feature_start(
            &project_id,
            &workflow_id,
            &title,
            &description,
            agent_kind.as_deref(),
            model.as_deref(),
            commit_artifacts,
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn feature_pause(ctx: State<'_, AppContext>, feature_id: String) -> Result<(), AppError> {
    ctx.executor
        .feature_pause(&feature_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn feature_resume(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<(), AppError> {
    ctx.executor
        .feature_resume(&feature_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn feature_cancel(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<(), AppError> {
    ctx.executor
        .feature_cancel(&feature_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn step_get(
    ctx: State<'_, AppContext>,
    execution_id: String,
) -> Result<StepExecution, AppError> {
    ctx.executor
        .step_get(&execution_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn step_list_for_run(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Vec<StepExecution>, AppError> {
    ctx.executor
        .step_list_for_run(&feature_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn gate_pending_for_run(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<GateDecision>, AppError> {
    ctx.presenter
        .gate_pending_for_run(&FeatureId::from(feature_id))
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn gate_decide(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), AppError> {
    ctx.presenter
        .gate_decide(
            &StepExecutionId::from(step_execution_id),
            &decision,
            feedback.as_deref(),
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn step_retry(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    new_model: Option<String>,
) -> Result<(), AppError> {
    ctx.executor
        .step_retry(&step_execution_id, new_model.as_deref())
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn replay_from_step(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    new_model: Option<String>,
) -> Result<(), AppError> {
    ctx.executor
        .replay_from_step(&step_execution_id, new_model.as_deref())
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn feature_get(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<Feature>, AppError> {
    ctx.features
        .get(&FeatureId::from(feature_id))
        .map_err(AppError::from)
}

/// Sync the feature branch with `origin/<default_branch>`. Returns
/// a `SyncOutcomeView` the UI can render directly:
/// - `Ok` when the merge was clean (or there was nothing to merge).
/// - `Conflict` when the merge left unmerged files; the UI offers a
///   "Resolve with agent" button that calls
///   `feature_resolve_sync_conflicts` with the same conflict list.
/// - `Resolved` after a successful agent resolution.
#[tauri::command]
pub async fn feature_sync(
    ctx: State<'_, AppContext>,
    feature_id: String,
    revalidate_step_execution_id: Option<String>,
) -> Result<SyncOutcomeView, AppError> {
    ctx.executor
        .feature_sync(&feature_id, revalidate_step_execution_id.as_deref())
        .await
        .map_err(AppError::from)
}

/// Spawn a fresh agent to resolve the conflicts left by
/// `feature_sync`. The agent edits the conflict files in a temporary
/// worktree, commits the resolution, and the worktree is merged back
/// into the feature branch. If `revalidate_step_execution_id` is set,
/// the named step is replayed so the workflow re-runs validation on
/// the freshly merged tree.
#[tauri::command]
pub async fn feature_resolve_sync_conflicts(
    ctx: State<'_, AppContext>,
    feature_id: String,
    conflict_files: Option<Vec<String>>,
    revalidate_step_execution_id: Option<String>,
) -> Result<SyncOutcomeView, AppError> {
    let files = conflict_files.unwrap_or_default();
    ctx.executor
        .feature_resolve_sync_conflicts(
            &feature_id,
            &files,
            revalidate_step_execution_id.as_deref(),
        )
        .await
        .map_err(AppError::from)
}
