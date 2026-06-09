use tauri::State;
use crate::state::{DatabaseState, AgentRegistryState, AgentConfigView};
use crate::domain::models::{AgentConfig, WorkingMemoryEntry};

#[tauri::command]
pub fn get_agent_configs(
    state: State<'_, DatabaseState>,
    registry_state: State<'_, AgentRegistryState>,
    machine_id: String,
) -> Result<Vec<AgentConfigView>, String> {
    let configured = state.db.get_agent_configs(&machine_id)?;
    let runtime_kinds: Vec<&'static str> = registry_state
        .registry
        .runtimes()
        .iter()
        .map(|r| r.kind())
        .collect();
    let mut views: Vec<AgentConfigView> = Vec::new();
    for cfg in configured {
        let available = runtime_kinds
            .iter()
            .find(|k| **k == cfg.kind)
            .map(|k| {
                registry_state
                    .registry
                    .runtime_for(k)
                    .map(|r| r.is_available(&machine_id))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        let install_command = runtime_kinds
            .iter()
            .find(|k| **k == cfg.kind)
            .and_then(|k| registry_state.registry.runtime_for(k).map(|r| r.install_command().to_string()))
            .unwrap_or_default();
        views.push(AgentConfigView {
            kind: cfg.kind,
            enabled: cfg.enabled,
            available,
            install_command,
        });
    }
    Ok(views)
}

#[tauri::command]
pub fn set_agent_configs(
    state: State<'_, DatabaseState>,
    machine_id: String,
    agents: Vec<AgentConfig>,
) -> Result<(), String> {
    let json = serde_json::to_string(&agents).map_err(|e| e.to_string())?;
    state.db.set_agent_configs(&machine_id, &json)
}

#[tauri::command]
pub fn get_working_memory(
    state: State<'_, DatabaseState>,
    thread_id: String,
) -> Result<Vec<WorkingMemoryEntry>, String> {
    state.db.get_working_memory(&thread_id)
}

#[tauri::command]
pub fn clear_working_memory(
    state: State<'_, DatabaseState>,
    thread_id: String,
) -> Result<(), String> {
    state.db.clear_working_memory(&thread_id)
}
