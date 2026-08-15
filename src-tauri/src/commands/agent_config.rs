use crate::domain::ids::MachineId;
use crate::domain::models::{AgentConfig, AgentKind, Availability, WorkingMemoryEntry};
use crate::error::AppError;
use crate::state::{AgentCatalogEntry, AgentConfigView, AppContext};
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

    let runtime_kinds: Vec<&'static str> =
        ctx.registry.runtimes().iter().map(|r| r.kind()).collect();

    let force = refresh.unwrap_or(false);
    let supported: Vec<&'static str> = runtime_kinds
        .iter()
        .copied()
        .filter(|k| AgentKind::is_supported(k))
        .collect();
    let known = ctx
        .registry
        .availability_of(&supported, &*ctx.exec, &machine_id, force)
        .await;

    // Merge in every registered, *supported* agent the stored config doesn't
    // know about yet. The DB persists only the enable/disable delta; the
    // registry is the source of truth for *which* agents exist. Without this,
    // an adapter added after a machine's config was last saved (e.g. codex on
    // a machine whose row predates it) would never appear in the settings
    // panel — the config list, not the registry, drove the view. What a
    // missing kind then defaults to is `AgentConfig::default_for`'s decision,
    // not this one's. Internal runtimes (noop / stub) are filtered out by
    // `is_supported` above.
    configured = AgentConfig::seed_missing(configured, &known);

    Ok(agent_config_views(&ctx.registry, configured, &known))
}

/// The pure half of [`get_agent_configs`] — configs plus probe results in,
/// rows for the settings table out — so the join between the two lists is
/// testable without an `AppContext` or a live probe.
///
/// A kind in `configured` with no entry in `known` is a stored config for an
/// agent this build no longer registers. It still gets a row, so a user can
/// see and clear it, but nothing claims it is available.
fn agent_config_views(
    registry: &demeteo_core::adapters::agent::registry::AgentRegistry,
    configured: Vec<AgentConfig>,
    known: &[(&str, Availability)],
) -> Vec<AgentConfigView> {
    configured
        .into_iter()
        .map(|cfg| {
            let available = known
                .iter()
                .find(|(k, _)| *k == cfg.kind)
                .is_some_and(|(_, availability)| availability.is_installed());
            let runtime = registry.runtime_for(&cfg.kind);
            let install_command = runtime
                .as_ref()
                .map(|r| r.install_command().to_string())
                .unwrap_or_default();
            let display_label = runtime
                .as_ref()
                .map(|r| r.capabilities().display_label.to_string())
                .unwrap_or_else(|| cfg.kind.clone());
            AgentConfigView {
                kind: cfg.kind,
                enabled: cfg.enabled,
                available,
                install_command,
                display_label,
            }
        })
        .collect()
}

/// The catalog of registered, user-selectable coding agents and the
/// capabilities Demeteo asks of each. The single source of truth the frontend
/// uses to populate agent pickers, replacing the hardcoded per-component
/// `AGENT_KINDS` lists. Internal runtimes (noop / stub) are excluded — only
/// kinds that are real supported agents are returned.
#[tauri::command]
pub fn list_agents(ctx: State<'_, AppContext>) -> Result<Vec<AgentCatalogEntry>, AppError> {
    Ok(agent_catalog(&ctx.registry))
}

/// The pure half of [`list_agents`] — registry in, catalog out — so the
/// mapping (notably the capability-driven `effort_levels`) is unit-testable
/// without an `AppContext`.
fn agent_catalog(
    registry: &demeteo_core::adapters::agent::registry::AgentRegistry,
) -> Vec<AgentCatalogEntry> {
    registry
        .runtimes()
        .iter()
        .filter(|r| AgentKind::is_supported(r.kind()))
        .map(|r| {
            let caps = r.capabilities();
            AgentCatalogEntry {
                kind: r.kind().to_string(),
                display_label: caps.display_label.to_string(),
                lists_models: caps.lists_models,
                default_model: caps.default_model.map(str::to_string),
                install_command: r.install_command().to_string(),
                // Straight from the runtime's own capabilities, so the picker
                // can never offer a level the agent would silently ignore.
                // Empty for hermes, which has no per-invocation effort control.
                effort_levels: caps.effort_levels.to_vec(),
                personalization: caps.personalization,
            }
        })
        .collect()
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

#[cfg(test)]
#[path = "../../tests/infrastructure/agent_config.rs"]
mod tests;
