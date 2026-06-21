use crate::domain::ids::MachineId;
use crate::domain::models::{AgentConfig, WorkingMemoryEntry};
use crate::state::{AgentConfigView, AppContext};
use tauri::State;

#[tauri::command]
pub fn get_agent_configs(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<Vec<AgentConfigView>, String> {
    let machine_id_typed = MachineId::from(machine_id.clone());
    // Resolve machine first using the same logic as RouterExecutionPort
    let resolved_id = match ctx.machines.get_machines() {
        Ok(machines) => machines
            .into_iter()
            .find(|m| {
                m.id == machine_id_typed
                    || format!("{}@{}", m.username, m.host) == machine_id
                    || m.host == machine_id
                    || m.name == machine_id
            })
            .map(|m| m.id)
            .unwrap_or(machine_id_typed),
        Err(_) => machine_id_typed,
    };

    let mut configured = ctx
        .threads
        .get_agent_configs(&resolved_id)
        .unwrap_or_else(|_| Vec::new());
    if configured.is_empty() {
        configured = vec![
            AgentConfig {
                kind: "opencode".to_string(),
                enabled: true,
            },
            AgentConfig {
                kind: "hermes".to_string(),
                enabled: true,
            },
            AgentConfig {
                kind: "claude-code".to_string(),
                enabled: true,
            },
            AgentConfig {
                kind: "antigravity".to_string(),
                enabled: true,
            },
        ];
    }

    let runtime_kinds: Vec<&'static str> =
        ctx.registry.runtimes().iter().map(|r| r.kind()).collect();
    let mut views: Vec<AgentConfigView> = Vec::new();
    for cfg in configured {
        let available = runtime_kinds
            .iter()
            .find(|k| **k == cfg.kind)
            .map(|k| ctx.registry.is_available(k, &*ctx.exec, &machine_id))
            .unwrap_or(false);
        let install_command = runtime_kinds
            .iter()
            .find(|k| **k == cfg.kind)
            .and_then(|k| {
                ctx.registry
                    .runtime_for(k)
                    .map(|r| r.install_command().to_string())
            })
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
    ctx: State<'_, AppContext>,
    machine_id: String,
    agents: Vec<AgentConfig>,
) -> Result<(), String> {
    let json = serde_json::to_string(&agents).map_err(|e| e.to_string())?;
    let machine_id_typed = MachineId::from(machine_id.clone());
    // Resolve machine first using the same logic as RouterExecutionPort
    let resolved_id = match ctx.machines.get_machines() {
        Ok(machines) => machines
            .into_iter()
            .find(|m| {
                m.id == machine_id_typed
                    || format!("{}@{}", m.username, m.host) == machine_id
                    || m.host == machine_id
                    || m.name == machine_id
            })
            .map(|m| m.id)
            .unwrap_or(machine_id_typed),
        Err(_) => machine_id_typed,
    };
    ctx.threads.set_agent_configs(&resolved_id, &json)
}

#[tauri::command]
pub fn get_working_memory(
    ctx: State<'_, AppContext>,
    thread_id: String,
) -> Result<Vec<WorkingMemoryEntry>, String> {
    ctx.threads
        .get_working_memory(&crate::domain::ids::ThreadId::from(thread_id))
}

#[tauri::command]
pub fn clear_working_memory(ctx: State<'_, AppContext>, thread_id: String) -> Result<(), String> {
    ctx.threads
        .clear_working_memory(&crate::domain::ids::ThreadId::from(thread_id))
}
