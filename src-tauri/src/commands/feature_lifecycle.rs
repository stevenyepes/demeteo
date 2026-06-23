//! Tauri commands for the completed-feature lifecycle (decision 26).
//! Tauri commands for the completed-feature lifecycle (decision 26).

use crate::application::lifecycle::CleanupResult;
use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub async fn feature_cleanup(
    ctx: State<'_, AppContext>,
    feature_id: String,
    force: Option<bool>,
) -> Result<CleanupResult, AppError> {
    crate::application::lifecycle::feature_cleanup(&ctx, feature_id, force)
        .await
        .map_err(AppError::from)
}
