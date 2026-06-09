use tauri::State;
use crate::state::AgentExecutionState;
use crate::domain::action::AgentAction;

#[tauri::command]
pub async fn request_action(
    state: State<'_, AgentExecutionState>,
    thread_id: String,
    machine_id: String,
    action: AgentAction,
) -> Result<crate::ports::agent_execution::CommandOutcome, String> {
    let exec = state.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.submit(&thread_id, &machine_id, action))
        .await
        .map_err(|e| format!("request_action join: {}", e))?
}

#[tauri::command]
pub async fn approve_intercept(
    state: State<'_, AgentExecutionState>,
    intercept_id: String,
) -> Result<(), String> {
    let exec = state.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.approve(&intercept_id))
        .await
        .map_err(|e| format!("approve_intercept join: {}", e))?
}

#[tauri::command]
pub async fn reject_intercept(
    state: State<'_, AgentExecutionState>,
    intercept_id: String,
    feedback: String,
) -> Result<(), String> {
    let exec = state.agent_exec.clone();
    tokio::task::spawn_blocking(move || exec.reject(&intercept_id, feedback))
        .await
        .map_err(|e| format!("reject_intercept join: {}", e))?
}
