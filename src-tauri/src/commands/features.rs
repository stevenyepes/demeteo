use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::ports::step_executor::SyncOutcomeView;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn fetch_active_features(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<Feature>, String> {
    ctx.features
        .get_active(&crate::domain::ids::ProjectId::from(project_id))
}

#[tauri::command]
pub async fn start_feature(
    ctx: State<'_, AppContext>,
    project_id: String,
    workflow_id: String,
    title: String,
    description: String,
    agent_kind: Option<String>,
    model: Option<String>,
) -> Result<Feature, String> {
    ctx.executor.feature_start(
        &project_id,
        &workflow_id,
        &title,
        &description,
        agent_kind.as_deref(),
        model.as_deref(),
    )
}

#[tauri::command]
pub fn feature_pause(ctx: State<'_, AppContext>, feature_id: String) -> Result<(), String> {
    ctx.executor.feature_pause(&feature_id)
}

#[tauri::command]
pub fn feature_resume(ctx: State<'_, AppContext>, feature_id: String) -> Result<(), String> {
    ctx.executor.feature_resume(&feature_id)
}

#[tauri::command]
pub fn feature_cancel(ctx: State<'_, AppContext>, feature_id: String) -> Result<(), String> {
    ctx.executor.feature_cancel(&feature_id)
}

#[tauri::command]
pub fn step_get(ctx: State<'_, AppContext>, execution_id: String) -> Result<StepExecution, String> {
    ctx.executor.step_get(&execution_id)
}

#[tauri::command]
pub fn step_list_for_run(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Vec<StepExecution>, String> {
    ctx.executor.step_list_for_run(&feature_id)
}

#[tauri::command]
pub fn gate_pending_for_run(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<GateDecision>, String> {
    ctx.presenter
        .gate_pending_for_run(&FeatureId::from(feature_id))
}

#[tauri::command]
pub async fn gate_decide(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), String> {
    ctx.presenter.gate_decide(
        &StepExecutionId::from(step_execution_id),
        &decision,
        feedback.as_deref(),
    )
}

#[tauri::command]
pub async fn step_retry(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    new_model: Option<String>,
) -> Result<(), String> {
    ctx.executor
        .step_retry(&step_execution_id, new_model.as_deref())
}

#[tauri::command]
pub async fn replay_from_step(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    new_model: Option<String>,
) -> Result<(), String> {
    ctx.executor
        .replay_from_step(&step_execution_id, new_model.as_deref())
}

#[tauri::command]
pub fn feature_get(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<Feature>, String> {
    ctx.features.get(&FeatureId::from(feature_id))
}

/// Sync the feature branch with `origin/<default_branch>`. Returns
/// a `SyncOutcomeView` the UI can render directly:
/// - `Ok` when the merge was clean (or there was nothing to merge).
/// - `Conflict` when the merge left unmerged files; the UI offers a
///   "Resolve with agent" button that calls
///   `feature_resolve_sync_conflicts` with the same conflict list.
/// - `Resolved` after a successful agent resolution.
#[tauri::command]
pub fn feature_sync(
    ctx: State<'_, AppContext>,
    feature_id: String,
    revalidate_step_execution_id: Option<String>,
) -> Result<SyncOutcomeView, String> {
    ctx.executor
        .feature_sync(&feature_id, revalidate_step_execution_id.as_deref())
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
) -> Result<SyncOutcomeView, String> {
    let files = conflict_files.unwrap_or_default();
    ctx.executor.feature_resolve_sync_conflicts(
        &feature_id,
        &files,
        revalidate_step_execution_id.as_deref(),
    )
}
