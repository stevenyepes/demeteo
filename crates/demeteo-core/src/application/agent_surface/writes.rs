//! Write operations for the `agent_surface` seam (module root docs). Each
//! delegates to an already-tested `application::*` function or port method —
//! this file adds no new business logic, only the narrower entry points an
//! external caller needs.

use crate::application::projects::{self, ProjectConfig};
use crate::application::tickets;
use crate::domain::ids::{MachineId, ProjectId, TicketId};
use crate::domain::models::project::RunShapePatch;
use crate::domain::models::{Feature, Project, ProjectSettings};
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

/// Register a project and its repository rows.
///
/// Registers rows only. The UI's create path goes on to
/// [`bootstrap_project`](crate::application::bootstrap::bootstrap_project)
/// (clone, worktree strategy) and saves settings; this deliberately does not —
/// a clone is long-running network and disk work that a `configure` caller
/// should not be able to start. The project is therefore left in status
/// `bootstrapping` with no settings row, and [`apply_run_shape_patch`] says so
/// rather than reporting it missing.
///
/// Every input is checked before the first insert, so a refusal leaves no
/// half-created project behind.
pub fn create_workspace_project(
    ctx: &AppContext,
    config: ProjectConfig,
) -> Result<Project, String> {
    if config.name.trim().is_empty() {
        return Err("name must not be empty".to_string());
    }
    match (config.compute_type.as_str(), config.remote_host.as_deref()) {
        ("local", None) => {}
        ("local", Some(_)) => {
            return Err("remote_host is only valid for compute_type `remote`".to_string())
        }
        ("remote", Some(host)) => {
            if ctx
                .machines
                .get_machine(&MachineId::from(host.to_string()))?
                .is_none()
            {
                return Err(format!("unknown remote_host `{host}`"));
            }
        }
        ("remote", None) => return Err("compute_type `remote` requires remote_host".to_string()),
        _ => return Err("compute_type must be `local` or `remote`".to_string()),
    }
    let providers = ctx.app_settings.get_provider_instances()?;
    for repo in &config.repos {
        if repo.repo_path.trim().is_empty() {
            return Err("repo_path must not be empty".to_string());
        }
        if !providers.iter().any(|p| p.id.as_str() == repo.provider_id) {
            return Err(format!("unknown provider_id `{}`", repo.provider_id));
        }
    }
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
/// return it. A patch that fails [`RunShapePatch::validate`] is refused before
/// anything is read or written.
pub fn apply_run_shape_patch(
    ctx: &AppContext,
    project_id: &ProjectId,
    patch: RunShapePatch,
) -> Result<ProjectSettings, String> {
    patch.validate().map_err(|e| e.to_string())?;
    if ctx.projects.get_project(project_id)?.is_none() {
        return Err("project not found".to_string());
    }
    let settings = ctx.projects.get_settings(project_id)?.ok_or_else(|| {
        "project has no settings yet: it has not been bootstrapped \
         (create_workspace_project registers rows only)"
            .to_string()
    })?;
    let settings = crate::domain::models::project::apply_run_shape_patch(settings, patch);
    ctx.projects.save_settings(settings.clone())?;
    Ok(settings)
}

#[cfg(test)]
#[path = "../../../tests/application/agent_surface_writes.rs"]
mod tests;
