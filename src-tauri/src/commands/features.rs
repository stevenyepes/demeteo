use tauri::State;
use crate::state::{DatabaseState, StepExecutorState};
use crate::domain::models::{Feature, StepExecution, GateDecision};

#[tauri::command]
pub fn fetch_active_features(
    state: State<'_, DatabaseState>,
    project_id: String,
) -> Result<Vec<Feature>, String> {
    state.db.get_active_features(&project_id)
}

#[tauri::command]
pub async fn start_feature(
    state: State<'_, StepExecutorState>,
    project_id: String,
    workflow_id: String,
    title: String,
) -> Result<Feature, String> {
    state.executor.feature_start(&project_id, &workflow_id, &title)
}

#[tauri::command]
pub fn feature_pause(
    state: State<'_, StepExecutorState>,
    feature_id: String,
) -> Result<(), String> {
    state.executor.feature_pause(&feature_id)
}

#[tauri::command]
pub fn feature_resume(
    state: State<'_, StepExecutorState>,
    feature_id: String,
) -> Result<(), String> {
    state.executor.feature_resume(&feature_id)
}

#[tauri::command]
pub fn feature_cancel(
    state: State<'_, StepExecutorState>,
    feature_id: String,
) -> Result<(), String> {
    state.executor.feature_cancel(&feature_id)
}

#[tauri::command]
pub fn step_list_for_run(
    state: State<'_, StepExecutorState>,
    feature_id: String,
) -> Result<Vec<StepExecution>, String> {
    state.executor.step_list_for_run(&feature_id)
}

#[tauri::command]
pub fn gate_pending_for_run(
    state: State<'_, StepExecutorState>,
    feature_id: String,
) -> Result<Option<GateDecision>, String> {
    state.presenter.gate_pending_for_run(&feature_id)
}

#[tauri::command]
pub async fn gate_decide(
    state: State<'_, StepExecutorState>,
    step_execution_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), String> {
    state.presenter.gate_decide(&step_execution_id, &decision, feedback.as_deref())
}

#[tauri::command]
pub async fn step_retry(
    state: State<'_, StepExecutorState>,
    step_execution_id: String,
) -> Result<(), String> {
    state.executor.step_retry(&step_execution_id)
}
