//! Write operations for the `agent_surface` seam (module root docs). Each
//! delegates to an already-tested `application::*` function or port method —
//! this file adds no new business logic, only the narrower entry points an
//! external caller needs.

use crate::application::projects::{self, ProjectConfig};
use crate::application::tickets;
use crate::domain::ids::{ProjectId, TicketId};
use crate::domain::models::project::RunShapePatch;
use crate::domain::models::{Feature, Project, ProjectSettings};
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

/// Create a project and its repositories.
pub fn create_workspace_project(
    ctx: &AppContext,
    config: ProjectConfig,
) -> Result<Project, String> {
    projects::create(ctx, config)
}

/// What an external caller chooses when starting a Feature directly, rather
/// than through a Ticket — deliberately narrower than [`FeatureLaunch`]:
/// agent/model/effort, step overrides, origin and budget are the run's own
/// business, not a caller's to set from outside.
pub struct AgentFeatureLaunch {
    pub project_id: String,
    pub workflow_id: String,
    pub title: String,
    pub description: String,
}

/// Start a Feature run. Returns as soon as the executor has accepted the
/// launch — the returned [`Feature`] is a handle, not a finished run.
pub async fn start_feature(
    ctx: &AppContext,
    launch: AgentFeatureLaunch,
) -> Result<Feature, String> {
    ctx.executor
        .feature_start(FeatureLaunch {
            project_id: launch.project_id,
            workflow_id: launch.workflow_id,
            title: launch.title,
            description: launch.description,
            ..FeatureLaunch::default()
        })
        .await
}

/// Start a Ticket's current attempt.
pub async fn start_ticket(ctx: &AppContext, ticket_id: &TicketId) -> Result<Feature, String> {
    tickets::launch::start(ctx, ticket_id).await
}

/// Apply a [`RunShapePatch`] to a project's settings, persist the result, and
/// return it.
pub fn apply_run_shape_patch(
    ctx: &AppContext,
    project_id: &ProjectId,
    patch: RunShapePatch,
) -> Result<ProjectSettings, String> {
    let settings = ctx
        .projects
        .get_settings(project_id)?
        .ok_or_else(|| "project not found".to_string())?;
    let settings = crate::domain::models::project::apply_run_shape_patch(settings, patch);
    ctx.projects.save_settings(settings.clone())?;
    Ok(settings)
}

#[cfg(test)]
#[path = "../../../tests/application/agent_surface_writes.rs"]
mod tests;
