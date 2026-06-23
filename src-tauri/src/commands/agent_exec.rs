use crate::domain::action::AgentAction;
use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub async fn request_action(
    ctx: State<'_, AppContext>,
    thread_id: String,
    machine_id: String,
    action: AgentAction,
) -> Result<crate::ports::agent_execution::CommandOutcome, AppError> {
    ctx.agent_exec
        .submit(&thread_id, &machine_id, action)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn approve_intercept(
    ctx: State<'_, AppContext>,
    intercept_id: String,
) -> Result<(), AppError> {
    ctx.agent_exec
        .approve(&intercept_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn reject_intercept(
    ctx: State<'_, AppContext>,
    intercept_id: String,
    feedback: String,
) -> Result<(), AppError> {
    ctx.agent_exec
        .reject(&intercept_id, feedback)
        .await
        .map_err(AppError::from)
}
