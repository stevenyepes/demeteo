//! Tauri commands for the completed-feature lifecycle (decision 26).
//! Tauri commands for the completed-feature lifecycle (decision 26).

use crate::application::cache_sweep::SweepReport;
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

#[tauri::command]
pub async fn feature_cache_sweep(
    ctx: State<'_, AppContext>,
    dry_run: bool,
) -> Result<SweepReport, AppError> {
    crate::application::cache_sweep::sweep_feature_caches(&ctx, dry_run)
        .await
        .map_err(AppError::from)
}
