//! Tauri commands for the in-app notification bell.
//!
//! Three thin wrappers around [`NotificationRepository`]: list the
//! recent panel, mark a single row read, and read the badge
//! unread-count. Heavy lifting (polling providers, persisting the
//! `MrMerged` row) lives in `adapters::mr_monitor`; this file is
//! pure delegation.

use crate::domain::ids::ProjectId;
use crate::domain::models::Notification;
use crate::error::AppError;
use tauri::State;

use crate::state::AppContext;

/// Maximum number of rows returned to the bell panel. The DB
/// indexes the most-recent 50, and the UI never needs more than
/// that for a dropdown.
const BELL_LIST_LIMIT: u32 = 50;

#[tauri::command]
pub async fn notifications_list(
    ctx: State<'_, AppContext>,
    project_id: Option<String>,
) -> Result<Vec<Notification>, AppError> {
    let pid = project_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(ProjectId::from);
    ctx.notifications
        .list(pid.as_ref(), BELL_LIST_LIMIT)
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn notification_mark_read(
    ctx: State<'_, AppContext>,
    id: String,
) -> Result<(), AppError> {
    ctx.notifications
        .mark_read(&id)
        .map(|_| ())
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn notification_unread_count(ctx: State<'_, AppContext>) -> Result<u32, AppError> {
    ctx.notifications.unread_count().map_err(AppError::from)
}
