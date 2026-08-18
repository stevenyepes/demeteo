use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::{
    EffortLevel, Feature, GateDecision, SequenceState, StepAttempt, StepExecution,
};
use crate::error::AppError;
use crate::ports::step_executor::{FeatureLaunch, SyncOutcomeView};
use crate::ports::sync_session::SyncSession;
use crate::state::AppContext;
use tauri::State;

// Re-exported so the wire shape the frontend already consumes is unchanged;
// the resolution logic now lives in `demeteo_core` so the runner's
// `get_worktree` control RPC shares exactly one code path with this command.
pub use crate::application::worktree::FeatureWorktreeInfo;

#[tauri::command]
pub async fn feature_get_worktree(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<FeatureWorktreeInfo, AppError> {
    crate::application::worktree::resolve_feature_worktree(&ctx, &FeatureId::from(feature_id))
        .await
        .map_err(AppError::from)
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

/// Pre-launch user attachments. Persisted to the feature row BEFORE
/// the driver is spawned so the agent's first turn sees them. Pass
/// `None` (or an empty array) when the user did not attach anything.
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
    // Feature-wide effort. Omitted (an older frontend) = `None` = inherit the
    // project default, which bottoms out at `EffortLevel::DEFAULT`. Per-step
    // efforts ride inside `step_overrides`, not here.
    effort: Option<EffortLevel>,
    commit_artifacts: Option<bool>,
    loop_iterations: Option<u32>,
    // Per-run dollar budget override (`--max-budget-usd`). Omitted (an older
    // frontend) = `None` = inherit the project default, then the engine
    // default.
    max_budget_usd: Option<f64>,
    step_overrides: Option<Vec<crate::domain::models::StepOverride>>,
    staged_attachments: Option<Vec<crate::commands::attachments::StagedAttachmentInput>>,
    // Omitted (a frontend older than the origin picker) = `None`, which
    // `FeatureLaunch::origin` and `FeatureLaunch::diff_base_branch` define.
    origin: Option<crate::domain::feature_origin::FeatureOrigin>,
    diff_base_branch: Option<String>,
) -> Result<Feature, AppError> {
    ctx.executor
        .feature_start(FeatureLaunch {
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
            step_overrides: step_overrides.unwrap_or_default(),
            staged_attachments: staged_attachments.unwrap_or_default(),
            origin: origin.unwrap_or_default(),
            diff_base_branch,
            ..FeatureLaunch::default()
        })
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
    // Read-model path (C3): display reads go through `RunView`, not the
    // executor, so a runner-owned step can later resolve from the shadow.
    ctx.run_view
        .step(&StepExecutionId::from(execution_id))
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found("Step execution not found".to_string()))
}

/// Per-attempt history for a step execution — the node drill-down panel's
/// Overview tab (P2.3). Read-model path (C3) like `step_get`: goes through
/// `RunView`, so a runner-owned step's attempts resolve from the shadow.
#[tauri::command]
pub async fn step_attempts_list(
    ctx: State<'_, AppContext>,
    execution_id: String,
) -> Result<Vec<StepAttempt>, AppError> {
    ctx.run_view
        .step_attempts(&StepExecutionId::from(execution_id))
        .map_err(AppError::from)
}

/// A `sequence` node's task list for the drill-down accordion (P2.5): each
/// task's landed-vs-pending state (Decision 13's committed prefix) plus its
/// per-task cost. Read-model path (C3) like `step_attempts_list`. `node_id` is
/// the graph node id (== v1 `step_id`); `execution_id` is its step-execution
/// row. A non-sequence or not-yet-planned node reads back `unplanned`.
#[tauri::command]
pub async fn sequence_tasks_list(
    ctx: State<'_, AppContext>,
    feature_id: String,
    node_id: String,
    execution_id: String,
) -> Result<SequenceState, AppError> {
    ctx.run_view
        .sequence_state(
            &FeatureId::from(feature_id),
            &node_id,
            &StepExecutionId::from(execution_id),
        )
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn step_list_for_run(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Vec<StepExecution>, AppError> {
    ctx.run_view
        .steps(&FeatureId::from(feature_id))
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
}

#[tauri::command]
pub async fn step_retry(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    new_model: Option<String>,
    new_agent: Option<String>,
    // Re-pin the feature-wide effort for the rerun, exactly as `new_model`
    // re-pins the model. `None` (or an older frontend that omits it) keeps
    // whatever the feature already carries.
    new_effort: Option<EffortLevel>,
) -> Result<(), AppError> {
    ctx.executor
        .step_retry(
            &step_execution_id,
            new_model.as_deref(),
            new_agent.as_deref(),
            new_effort,
        )
        .await
}

#[tauri::command]
pub async fn replay_from_step(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    new_model: Option<String>,
    new_agent: Option<String>,
    new_effort: Option<EffortLevel>,
) -> Result<(), AppError> {
    ctx.executor
        .replay_from_step(
            &step_execution_id,
            new_model.as_deref(),
            new_agent.as_deref(),
            new_effort,
        )
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn feature_get(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<Feature>, AppError> {
    ctx.run_view
        .feature(&FeatureId::from(feature_id))
        .map_err(AppError::from)
}

/// The UTF-8 body of a run's declared artifact. This is a *display* read of a
/// run surface, so it goes through `RunView` (C3) rather than `sftp_read_file`
/// directly — that is the seam C4 uses to serve a runner-owned feature's
/// artifact from the lazily-cached laptop shadow. General filesystem browsing
/// (the code editor) stays on `sftp_read_file`; only run-artifact display uses
/// this.
#[tauri::command]
pub async fn artifact_body(
    ctx: State<'_, AppContext>,
    machine_id: String,
    path: String,
) -> Result<String, AppError> {
    ctx.run_view
        .artifact_body(&machine_id, &path)
        .await
        .map_err(AppError::from)
}

/// Sync the feature branch with `origin/<default_branch>`. Returns
/// a `SyncOutcomeView` the UI can render directly:
/// - `Ok` when the merge was clean (or there was nothing to merge).
/// - `Conflict` when the merge left unmerged files; the UI offers a
///   "Resolve with agent" button that calls
///   `feature_resolve_sync_conflicts` with the same conflict list.
/// - `Blocked` when the sync stopped short of a merge; there is
///   nothing for an agent to resolve.
/// - `Resolved` after a successful agent resolution.
#[tauri::command]
pub async fn feature_sync(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<SyncOutcomeView, AppError> {
    ctx.executor
        .feature_sync(&feature_id)
        .await
        .map_err(AppError::from)
}

/// The feature's live sync, or `null` if it has never synced.
///
/// Reconciled against the working tree before it answers, so a session left
/// `resolving` by a process that died — or a conflict whose worktree a later
/// sync force-removed — is never handed to the UI as if it were still true.
#[tauri::command]
pub async fn sync_session_get(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<SyncSession>, AppError> {
    crate::application::sync_session::get_reconciled(
        &ctx.sync_sessions,
        &ctx.exec,
        &FeatureId::from(feature_id),
    )
    .await
    .map_err(AppError::from)
}

/// Give up on the feature's sync: undo the merge, discard the worktree, and
/// close the session. Safe on a worktree that is already gone, which is the
/// common case.
#[tauri::command]
pub async fn sync_abort(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<Option<SyncSession>, AppError> {
    crate::application::sync_session::abort(
        &ctx.sync_sessions,
        &ctx.exec,
        &FeatureId::from(feature_id),
    )
    .await
    .map_err(AppError::from)
}

/// Spawn a fresh agent to resolve the conflicts left by
/// `feature_sync`. The agent edits the conflict files in a temporary
/// worktree, commits the resolution, and the worktree is merged back
/// into the feature branch.
#[tauri::command]
pub async fn feature_resolve_sync_conflicts(
    ctx: State<'_, AppContext>,
    feature_id: String,
    conflict_files: Option<Vec<String>>,
) -> Result<SyncOutcomeView, AppError> {
    let files = conflict_files.unwrap_or_default();
    ctx.executor
        .feature_resolve_sync_conflicts(&feature_id, &files)
        .await
        .map_err(AppError::from)
}
