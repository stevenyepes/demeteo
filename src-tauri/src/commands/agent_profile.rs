use tauri::State;
use crate::state::DatabaseState;
use crate::domain::models::AgentProfile;

#[tauri::command]
pub fn get_agent_profiles(state: State<'_, DatabaseState>, machine_id: String) -> Result<Vec<AgentProfile>, String> {
    state.db.get_agent_profiles(&machine_id)
}

#[tauri::command]
pub fn add_agent_profile(state: State<'_, DatabaseState>, profile: AgentProfile) -> Result<(), String> {
    state.db.add_agent_profile(profile)
}

#[tauri::command]
pub fn delete_agent_profile(state: State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_agent_profile(&id)
}
