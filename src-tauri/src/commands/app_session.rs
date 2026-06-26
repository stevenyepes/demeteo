use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

const WORKSPACE_BASE_DIR_KEY: &str = "workspace_base_dir";

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
