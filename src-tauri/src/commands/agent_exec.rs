use crate::domain::action::AgentAction;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub async fn request_action(
    ctx: State<'_, AppContext>,
    thread_id: String,
    machine_id: String,
    action: AgentAction,
) -> Result<crate::ports::agent_execution::CommandOutcome, String> {
    let exec = ctx.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.submit(&thread_id, &machine_id, action))
        .await
        .map_err(|e| format!("request_action join: {}", e))?
}

#[tauri::command]
pub async fn approve_intercept(
    ctx: State<'_, AppContext>,
    intercept_id: String,
) -> Result<(), String> {
    let exec = ctx.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.approve(&intercept_id))
        .await
        .map_err(|e| format!("approve_intercept join: {}", e))?
}

#[tauri::command]
pub async fn reject_intercept(
    ctx: State<'_, AppContext>,
    intercept_id: String,
    feedback: String,
) -> Result<(), String> {
    let exec = ctx.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.reject(&intercept_id, feedback))
        .await
        .map_err(|e| format!("reject_intercept join: {}", e))?
}
