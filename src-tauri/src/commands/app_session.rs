use crate::error::AppError;
use crate::ports::db::AppSettingsRepository;
use crate::state::AppContext;
use tauri::State;

const WORKSPACE_BASE_DIR_KEY: &str = "workspace_base_dir";
const RUN_IN_BACKGROUND_KEY: &str = "run_in_background";

/// Interpret the stored `run_in_background` app_session value as a bool.
///
/// Pure so it can be unit-tested without a live `AppContext`: an absent key
/// (or any value other than the string `"true"`) is treated as `false`.
fn parse_run_in_background(value: Option<String>) -> bool {
    matches!(value.as_deref(), Some("true"))
}

/// Command core for [`get_run_in_background`], factored out to take the settings
/// port directly so an integration test can exercise the *exact* key + decode
/// against a real store without constructing an `AppContext` (whose ~25-port
/// constructor is Tauri-internal). Returns `false` when the key is absent.
pub fn read_run_in_background(store: &dyn AppSettingsRepository) -> Result<bool, String> {
    store
        .get_app_session(RUN_IN_BACKGROUND_KEY)
        .map(parse_run_in_background)
}

/// Command core for [`set_run_in_background`]: persist the preference as the
/// canonical `"true"`/`"false"` string. Shared with the integration test so the
/// encoding under real test is the one the command actually writes.
pub fn write_run_in_background(
    store: &dyn AppSettingsRepository,
    enabled: bool,
) -> Result<(), String> {
    let value = if enabled { "true" } else { "false" };
    store.set_app_session(RUN_IN_BACKGROUND_KEY, value)
}

/// Identity bundle returned to the frontend on startup.
///
/// The frontend renders these on the About screen: `version` comes from
/// `CARGO_PKG_VERSION` (always matches `tauri.conf.json`); `channel` comes
/// from the compile-time `DEMETEO_RELEASE_CHANNEL` env var, defaulting to
/// `"stable"`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AppInfo {
    pub version: String,
    pub channel: String,
}

#[tauri::command]
pub fn get_app_session(
    ctx: State<'_, AppContext>,
    key: String,
) -> Result<Option<String>, AppError> {
    ctx.app_settings
        .get_app_session(&key)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_app_session(
    ctx: State<'_, AppContext>,
    key: String,
    value: String,
) -> Result<(), AppError> {
    ctx.app_settings
        .set_app_session(&key, &value)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn delete_app_session(ctx: State<'_, AppContext>, key: String) -> Result<(), AppError> {
    ctx.app_settings
        .delete_app_session(&key)
        .map_err(AppError::from)
}

/// Returns the binary's version + release channel for the About screen.
///
/// Pure constants — no `State` lookup required.
#[tauri::command]
pub fn get_app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        channel: crate::RELEASE_CHANNEL.to_string(),
    }
}

/// Returns the **effective** workspace directory currently in use.
/// This is the resolved value (override if set, otherwise app data dir).
#[tauri::command]
pub fn get_workspace_dir(ctx: State<'_, AppContext>) -> String {
    ctx.workspace_dir.to_string_lossy().to_string()
}

/// Returns the stored workspace directory override (or `None` if using default).
#[tauri::command]
pub fn get_workspace_dir_setting(ctx: State<'_, AppContext>) -> Result<Option<String>, AppError> {
    ctx.app_settings
        .get_app_session(WORKSPACE_BASE_DIR_KEY)
        .map(|v| v.filter(|s| !s.trim().is_empty()))
        .map_err(AppError::from)
}

/// Persist a workspace directory override.
///
/// Pass `None` (or an empty string) to clear the override and revert to
/// the default app data directory. The change takes effect after restarting
/// the app; existing projects remain in their current location until
/// re-bootstrapped.
#[tauri::command]
pub fn set_workspace_dir_setting(
    ctx: State<'_, AppContext>,
    path: Option<String>,
) -> Result<(), AppError> {
    let value = path
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_default();
    ctx.app_settings
        .set_app_session(WORKSPACE_BASE_DIR_KEY, &value)
        .map_err(AppError::from)
}

/// Read the `run_in_background` preference.
///
/// Returns `false` when the key was never set; otherwise parses the stored
/// `"true"`/`"false"` string (see [`parse_run_in_background`]).
#[tauri::command]
pub fn get_run_in_background(ctx: State<'_, AppContext>) -> Result<bool, AppError> {
    read_run_in_background(ctx.app_settings.as_ref()).map_err(AppError::from)
}

/// Persist the `run_in_background` preference as the string `"true"`/`"false"`.
#[tauri::command]
pub fn set_run_in_background(ctx: State<'_, AppContext>, enabled: bool) -> Result<(), AppError> {
    write_run_in_background(ctx.app_settings.as_ref(), enabled).map_err(AppError::from)
}

#[cfg(test)]
#[path = "../../tests/infrastructure/app_session.rs"]
mod tests;
