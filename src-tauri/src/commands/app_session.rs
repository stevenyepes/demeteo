use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

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
