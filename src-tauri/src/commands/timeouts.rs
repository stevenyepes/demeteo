//! Tauri commands for the global agent-turn timeout settings.
//!
//! The values are stored as JSON in the `app_settings` KV table under the
//! `agent_timeouts` key (see [`crate::application::timeouts::CONFIG_KEY`]).
//! All call sites read the effective values through
//! [`crate::application::timeouts::resolve_effective`] at turn start.

use tauri::State;

use crate::application::timeouts as timeouts_app;
use crate::domain::models::AgentTimeouts;
use crate::error::AppError;
use crate::state::AppContext;

/// Return the persisted agent timeouts (defaults when unset / unparseable).
#[tauri::command]
pub fn get_agent_timeouts(ctx: State<'_, AppContext>) -> Result<AgentTimeouts, AppError> {
    Ok(timeouts_app::resolve_effective(ctx.app_settings.as_ref()))
}

/// Persist a new agent timeouts configuration. The input is validated
/// (monotonicity + bounds) before being written; out-of-range input
/// surfaces as an `AppError::Validation`.
#[tauri::command]
pub fn set_agent_timeouts(
    ctx: State<'_, AppContext>,
    config: AgentTimeouts,
) -> Result<(), AppError> {
    let validated = AgentTimeouts::validated(
        config.fast_timeout_s,
        config.normal_timeout_s,
        config.wall_cap_s,
    )
    .map_err(AppError::validation)?;
    timeouts_app::save(ctx.app_settings.as_ref(), &validated).map_err(AppError::from)
}
