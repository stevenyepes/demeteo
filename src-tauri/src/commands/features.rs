use tauri::State;
use crate::state::AppContext;
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::{Feature, StepExecution, GateDecision};

#[tauri::command]
pub fn fetch_active_features(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<Feature>, String> {
    ctx.features.get_active(&crate::domain::ids::ProjectId::from(project_id))
}

#[tauri::command]
pub async fn start_feature(
    ctx: State<'_, AppContext>,
    project_id: String,
    workflow_id: String,
    title: String,
) -> Result<Feature, String> {
    ctx.executor.feature_start(&project_id, &workflow_id, &title)
}

#[tauri::command]
pub fn feature_pause(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<(), String> {
    ctx.executor.feature_pause(&feature_id)
}

#[tauri::command]
pub fn feature_resume(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<(), String> {
    ctx.executor.feature_resume(&feature_id)
}

#[tauri::command]
pub fn feature_cancel(
    ctx: State<'_, AppContext>,
    feature_id: String,
) -> Result<(), String> {
    ctx.executor.feature_cancel(&feature_id)
}

#[tauri::command]
pub fn step_get(
    ctx: State<'_, AppContext>,
    execution_id: String,
) -> Result<StepExecution, String> {
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
    ctx.presenter.gate_pending_for_run(&FeatureId::from(feature_id))
}

#[tauri::command]
pub async fn gate_decide(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), String> {
    ctx.presenter.gate_decide(&StepExecutionId::from(step_execution_id), &decision, feedback.as_deref())
}

#[tauri::command]
pub async fn step_retry(
    ctx: State<'_, AppContext>,
    step_execution_id: String,
) -> Result<(), String> {
    ctx.executor.step_retry(&step_execution_id)
}
