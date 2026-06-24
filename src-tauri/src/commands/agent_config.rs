use crate::domain::ids::MachineId;
use crate::domain::models::{AgentConfig, WorkingMemoryEntry};
use crate::error::AppError;
use crate::state::{AgentConfigView, AppContext};
use tauri::State;

#[tauri::command]
pub async fn get_agent_configs(
    ctx: State<'_, AppContext>,
    machine_id: String,
    // When true, the availability probe is run fresh for each agent and
    // the in-memory cache is updated with the new result. The settings
    // page's "Re-check" button passes `true`; everything else uses
    // `false` to avoid re-probing on every list.
    refresh: Option<bool>,
) -> Result<Vec<AgentConfigView>, AppError> {
    let resolved_id = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )
    .map(|m| m.id)
    .unwrap_or_else(|_| MachineId::from(machine_id.clone()));

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
    let force = refresh.unwrap_or(false);
    for cfg in configured {
        let available = match runtime_kinds.iter().find(|k| **k == cfg.kind) {
            Some(k) => {
                ctx.registry
                    .is_available(k, &*ctx.exec, &machine_id, force)
                    .await
            }
            None => false,
        };
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
) -> Result<(), AppError> {
    let json = serde_json::to_string(&agents).map_err(|e| AppError::from(e.to_string()))?;
    let resolved_id = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        &*ctx.machines,
        &machine_id,
    )
    .map(|m| m.id)
    .unwrap_or_else(|_| MachineId::from(machine_id.clone()));
    ctx.threads
        .set_agent_configs(&resolved_id, &json)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_working_memory(
    ctx: State<'_, AppContext>,
    thread_id: String,
) -> Result<Vec<WorkingMemoryEntry>, AppError> {
    ctx.threads
        .get_working_memory(&crate::domain::ids::ThreadId::from(thread_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn clear_working_memory(ctx: State<'_, AppContext>, thread_id: String) -> Result<(), AppError> {
    ctx.threads
        .clear_working_memory(&crate::domain::ids::ThreadId::from(thread_id))
        .map_err(AppError::from)
}
