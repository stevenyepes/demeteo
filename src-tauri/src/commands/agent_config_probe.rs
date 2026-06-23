use crate::domain::models::ConfigOptionValue;
use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub async fn get_agent_models(
    ctx: State<'_, AppContext>,
    machine_id: String,
    agent_kind: String,
) -> Result<Vec<ConfigOptionValue>, AppError> {
    crate::application::agent_probe::discover_models(&ctx, machine_id, agent_kind)
        .await
        .map_err(AppError::from)
}
