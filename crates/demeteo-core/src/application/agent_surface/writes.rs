//! Write operations for the `agent_surface` seam (module root docs). Each
//! delegates to an already-tested `application::*` function or port method —
//! this file adds no new business logic, only the narrower entry points an
//! external caller needs.

pub use crate::application::attachments::AgentAttachment;
use crate::application::attachments::{
    agent_attachment_forbidden_roots, resolve_agent_attachments,
};
use crate::application::launch::{launch_run, LaunchRequest, LaunchedRun};
use crate::application::projects::{self, ProjectConfig};
use crate::application::tickets;
use crate::domain::feature_launch::refuse_blank_launch_text;
use crate::domain::ids::{MachineId, ProjectId, TicketId, WorkflowId};
use crate::domain::models::project::RunShapePatch;
use crate::domain::models::{Project, ProjectSettings};
use crate::domain::run_placement::{placement_from_raw, placement_override, DetachedOptions};
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
/// agent/model/effort, step overrides and origin are the project's run shape,
/// not a caller's to set from outside. Attachments are the caller's input.
///
/// The caller does choose *where* the run goes. An absent, blank or
/// local-meaning `machine_id` is local and any other id is detached, through
/// [`placement_from_raw`]. The four remaining fields are
/// [`DetachedOptions`]: the caps only bound spend on a detached run, which is
/// always unattended, and sending any of them with a local placement is
/// refused rather than dropped ([`crate::domain::run_placement`]).
pub struct AgentFeatureLaunch {
    pub project_id: String,
    pub workflow_id: String,
    pub title: String,
    pub description: String,
    pub attachments: Vec<AgentAttachment>,
    pub machine_id: Option<String>,
    pub target_repo_id: Option<String>,
    pub unattended: Option<bool>,
    pub max_cost_usd: Option<f64>,
    pub max_wall_clock_secs: Option<u64>,
}

/// Start a Feature run. Returns as soon as the run was accepted — by the
/// executor, or by the machine's runner — so the returned
/// [`LaunchedRun`]'s Feature is a handle, not a finished run.
pub async fn start_feature(
    ctx: &AppContext,
    launch: AgentFeatureLaunch,
) -> Result<LaunchedRun, String> {
    // Blank text is refused before any file is read, with the words
    // `feature_start` uses. `feature_start` inserts the bootstrapping Feature
    // row before it stages anything, so an attachment refusal has to happen
    // here, ahead of the launch.
    refuse_blank_launch_text(&launch.title, &launch.description)?;
    let staged_attachments = if launch.attachments.is_empty() {
        Vec::new()
    } else {
        // A bad project or workflow is refused before a byte is read. Only on
        // this path: `feature_start` itself does not refuse either up front,
        // and a launch without attachments must keep behaving as it did.
        if ctx
            .projects
            .get_project(&ProjectId::from(launch.project_id.clone()))?
            .is_none()
        {
            return Err("project not found".to_string());
        }
        if ctx
            .workflows
            .get(&WorkflowId::from(launch.workflow_id.clone()))?
            .is_none()
        {
            return Err("workflow not found".to_string());
        }
        let items = launch.attachments;
        let forbidden = agent_attachment_forbidden_roots(&ctx.app_data_dir, &ctx.workspace_dir);
        tokio::task::spawn_blocking(move || resolve_agent_attachments(items, &forbidden))
            .await
            .map_err(|e| e.to_string())??
    };
    let placement = placement_from_raw(launch.machine_id.as_deref());
    let request = LaunchRequest {
        launch: FeatureLaunch {
            project_id: launch.project_id,
            workflow_id: launch.workflow_id,
            title: launch.title,
            description: launch.description,
            staged_attachments,
            ..FeatureLaunch::default()
        },
        placement,
        detached: DetachedOptions {
            target_repo_id: launch.target_repo_id,
            unattended: launch.unattended,
            max_cost_usd: launch.max_cost_usd,
            max_wall_clock_secs: launch.max_wall_clock_secs,
        },
    };
    launch_run(ctx, request).await.map_err(|e| e.to_string())
}

/// Start a Ticket's current attempt.
///
/// `machine_id` places this one launch and is never stored on the ticket
/// ([`tickets::launch::start`]). An absent or blank one is no override — the
/// ticket's own placement applies — whereas `"local"` overrides to local.
pub async fn start_ticket(
    ctx: &AppContext,
    ticket_id: &TicketId,
    machine_id: Option<String>,
) -> Result<LaunchedRun, String> {
    tickets::launch::start(ctx, ticket_id, placement_override(machine_id.as_deref()))
        .await
        .map_err(|e| e.to_string())
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
