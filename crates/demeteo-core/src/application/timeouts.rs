//! Application service for global agent-turn timeouts.
//!
//! Timeouts are stored as JSON in the `app_settings` KV table under the
//! [`CONFIG_KEY`] key. The shape is [`AgentTimeouts`]; the resolver
//! [`resolve_effective`] returns the configured values or the built-in
//! defaults when nothing is persisted yet.
//!
//! Every agent-turn call site (planner, worker, resolver, verifier, agent
//! step) reads its values through [`resolve_effective`]. This is the single
//! wiring point — change it here once and every turn honors the new value.

use crate::domain::models::{AgentTimeouts, CONFIG_KEY};
use crate::ports::db::AppSettingsRepository;

/// Load the persisted config, falling back to [`AgentTimeouts::default`] when
/// unset or unparseable. Failures (missing key, malformed JSON) all collapse
/// to the default so a corrupt row can never block an agent turn.
pub fn load(app_settings: &dyn AppSettingsRepository) -> AgentTimeouts {
    match app_settings.app_setting_get(CONFIG_KEY) {
        Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_default(),
        _ => AgentTimeouts::default(),
    }
}

/// Persist the config as JSON.
pub fn save(
    app_settings: &dyn AppSettingsRepository,
    config: &AgentTimeouts,
) -> Result<(), String> {
    let json = serde_json::to_string(config).map_err(|e| e.to_string())?;
    app_settings.app_setting_set(CONFIG_KEY, &json)
}

/// Resolve the effective timeouts for one agent turn. Pulls the persisted
/// config (or defaults) and validates the values; an out-of-range persisted
/// row falls back to defaults rather than blocking the run. This is the
/// function every call site should use.
pub fn resolve_effective(app_settings: &dyn AppSettingsRepository) -> AgentTimeouts {
    let loaded = load(app_settings);
    sanitize(loaded)
}

/// Clamp an arbitrary [`AgentTimeouts`] into the safe operating envelope.
/// Used to recover from a hand-edited `app_settings` row that violates
/// monotonicity (`normal ≥ fast`, `wall ≥ normal`) or the hard caps.
fn sanitize(t: AgentTimeouts) -> AgentTimeouts {
    AgentTimeouts::validated(
        t.fast_timeout_s.clamp(10, 3600),
        t.normal_timeout_s.clamp(10, 7200).max(t.fast_timeout_s),
        t.wall_cap_s.clamp(10, 14400).max(t.normal_timeout_s),
    )
    .unwrap_or_default()
}

#[cfg(test)]
#[path = "../../tests/application/timeouts.rs"]
mod tests;
