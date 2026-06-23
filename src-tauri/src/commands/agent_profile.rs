use crate::domain::ids::MachineId;
use crate::domain::models::AgentProfile;
use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn get_agent_profiles(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<Vec<AgentProfile>, AppError> {
    ctx.machines
        .get_agent_profiles(&MachineId::from(machine_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn add_agent_profile(
    ctx: State<'_, AppContext>,
    profile: AgentProfile,
) -> Result<(), AppError> {
    ctx.machines
        .add_agent_profile(profile)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn delete_agent_profile(ctx: State<'_, AppContext>, id: String) -> Result<(), AppError> {
    ctx.machines
        .delete_agent_profile(&crate::domain::ids::AgentProfileId::from(id))
        .map_err(AppError::from)
}
