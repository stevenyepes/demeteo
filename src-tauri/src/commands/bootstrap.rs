use crate::domain::ids::ProjectId;
use crate::domain::models::{ProjectSettings, WorktreeStrategy};
use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub async fn bootstrap_project(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<WorktreeStrategy, AppError> {
    crate::application::bootstrap::bootstrap_project(&ctx, project_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_proposed_strategy(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Option<ProjectSettings>, AppError> {
    ctx.projects
        .get_settings(&ProjectId::from(project_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn save_project_settings(
    ctx: State<'_, AppContext>,
    project_id: String,
    settings: ProjectSettings,
) -> Result<(), AppError> {
    let project_id_typed = ProjectId::from(project_id);
    // Save to DB
    ctx.projects
        .save_settings(settings)
        .map_err(AppError::from)?;

    // Set project status to idle (workspace build complete)
    ctx.projects
        .update_status(&project_id_typed, "idle")
        .map_err(AppError::from)?;

    Ok(())
}
