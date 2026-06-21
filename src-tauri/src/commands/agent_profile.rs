use crate::domain::ids::MachineId;
use crate::domain::models::AgentProfile;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn get_agent_profiles(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<Vec<AgentProfile>, String> {
    ctx.machines
        .get_agent_profiles(&MachineId::from(machine_id))
}

#[tauri::command]
pub fn add_agent_profile(ctx: State<'_, AppContext>, profile: AgentProfile) -> Result<(), String> {
    ctx.machines.add_agent_profile(profile)
}

#[tauri::command]
pub fn delete_agent_profile(ctx: State<'_, AppContext>, id: String) -> Result<(), String> {
    ctx.machines
        .delete_agent_profile(&crate::domain::ids::AgentProfileId::from(id))
}
