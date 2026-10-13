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
use crate::application::step_control;
use crate::application::tickets;
use crate::domain::feature_launch::refuse_blank_launch_text;
use crate::domain::gate::critic_rework::{
    self, ReworkRequest, CRITIC_ARTIFACT, MAX_CLIENT_REWORKS,
};
use crate::domain::ids::{
    FeatureId, MachineId, ProjectId, StepExecutionId, StepId, TicketId, WorkflowId,
};
use crate::domain::models::project::RunShapePatch;
use crate::domain::models::{
    EffortLevel, Feature, Project, ProjectSettings, StepConfig, StepExecution, StepOverride,
};
use crate::domain::run_placement::{placement_from_raw, placement_override, DetachedOptions};
use crate::domain::step_assignment::{identifier_refusal, StepAssignment};
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

/// What an external caller chooses when starting a Ticket. The run shape is
/// the ticket's own; the caller picks where this one launch goes and what
/// bounds it ([`tickets::launch::StartBounds`]).
///
/// `machine_id` places this one launch and is never stored on the ticket. An
/// absent or blank one is no override — the ticket's own placement applies —
/// whereas `"local"` overrides to local.
pub struct AgentTicketLaunch {
    pub ticket_id: String,
    pub machine_id: Option<String>,
    pub max_cost_usd: Option<f64>,
    pub max_wall_clock_secs: Option<u64>,
    pub max_in_flight: Option<usize>,
}

/// Start a Ticket's current attempt.
pub async fn start_ticket(
    ctx: &AppContext,
    launch: AgentTicketLaunch,
) -> Result<LaunchedRun, String> {
    tickets::launch::start_bounded(
        ctx,
        &TicketId::from(launch.ticket_id),
        placement_override(launch.machine_id.as_deref()),
        tickets::launch::StartBounds {
            max_cost_usd: launch.max_cost_usd,
            max_wall_clock_secs: launch.max_wall_clock_secs,
            max_in_flight: launch.max_in_flight,
        },
    )
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

/// Re-run one step that failed, was interrupted, or never started, on the
/// agent and model it already resolves to. Returns the step as it stands once
/// the retry was accepted — a handle, not a finished attempt.
///
/// Replay is not offered beside it: it takes a step of any status, and on a
/// live run it kills the run first.
pub async fn retry_step(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
) -> Result<StepExecution, String> {
    step_control::retry_step(ctx, step_execution_id)
        .await
        .map_err(|e| e.to_string())?;
    ctx.features
        .step_get(step_execution_id)?
        .ok_or_else(|| "step execution not found".to_string())
}

/// What an external caller pins one step to. `effort` is the wire spelling
/// and is refused, not defaulted, when it names no level.
pub struct AgentStepAssignment {
    pub step_execution_id: String,
    pub agent_kind: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
}

/// The pins a Feature carries once an assignment was written.
#[derive(Debug, serde::Serialize)]
pub struct StepAssignmentOutcome {
    pub feature_id: FeatureId,
    pub step_id: StepId,
    pub step_overrides: Vec<StepOverride>,
}

/// Pin which agent, model and effort one step of a started run uses. The
/// three fields replace the step's pin as a whole; all three absent removes it.
///
/// The launch surface leaves agent, model and effort to the project's run
/// shape ([`AgentFeatureLaunch`]). This is the exception after launch, and it
/// stays inside what `spend` already prices: it changes what one step of one
/// run costs and writes nothing a later run reads.
pub async fn set_step_assignment(
    ctx: &AppContext,
    change: AgentStepAssignment,
) -> Result<StepAssignmentOutcome, String> {
    let effort = change
        .effort
        .as_deref()
        .map(|raw| {
            EffortLevel::parse(raw)
                .ok_or_else(|| "effort must be one of low, medium, high, xhigh, max".to_string())
        })
        .transpose()?;
    let assignment = StepAssignment {
        agent_kind: change.agent_kind,
        model: change.model,
        effort,
    };
    if let Some(refusal) = identifier_refusal(&assignment) {
        return Err(refusal);
    }
    let step_execution_id = StepExecutionId::from(change.step_execution_id);
    step_control::set_step_assignment(ctx, &step_execution_id, assignment)
        .await
        .map_err(|e| e.to_string())?;
    let step = ctx
        .features
        .step_get(&step_execution_id)?
        .ok_or_else(|| "step execution not found".to_string())?;
    let feature = ctx
        .features
        .get(&step.feature_id)?
        .ok_or_else(|| "feature not found".to_string())?;
    Ok(StepAssignmentOutcome {
        feature_id: feature.id,
        step_id: step.step_id,
        step_overrides: feature.step_overrides,
    })
}

/// What [`rework_from_critic`] did.
#[derive(Debug, serde::Serialize)]
pub struct CriticRework {
    pub feature_id: FeatureId,
    /// The critic's verdict the rework was opened on.
    pub verdict: &'static str,
    pub reworks_used: usize,
    pub reworks_remaining: usize,
}

/// The run-event kind that records one accepted [`rework_from_critic`].
///
/// Counted to enforce the cap. The redirect itself cannot be counted: it
/// deletes its own `gate_decisions` row, and a detached run's `gate_decided`
/// event is written on the runner, never into this log.
pub const CRITIC_REWORK_EVENT: &str = "critic_rework_requested";

/// Send the Standard workflow's final gate back to fix what its critic found.
///
/// The caller names the gate and supplies no text: the feedback is composed
/// by [`critic_rework::plan`] from the stored review, which is read by step
/// from the artifact store and never from a path the caller could choose.
/// This can only redirect. It is not a route to approving a gate.
pub async fn rework_from_critic(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
) -> Result<CriticRework, String> {
    let gate = ctx
        .features
        .step_get(step_execution_id)?
        .ok_or_else(|| "step execution not found".to_string())?;
    let feature = ctx
        .features
        .get(&gate.feature_id)?
        .ok_or_else(|| "feature not found".to_string())?;
    let steps = pinned_steps(ctx, &feature)?;
    let workflow_id = feature.workflow_id.as_ref().map(|id| id.as_ref());
    let critic =
        critic_rework::critic_step(workflow_id, &steps, &gate).map_err(|e| e.to_string())?;
    let executions = ctx.features.steps_for_feature(&feature.id)?;
    let review = critic_review(
        ctx,
        &feature.id,
        &critic_rework::review_store_keys(&executions, critic),
    )?;
    let prior_reworks = ctx
        .run_events
        .list_since(feature.id.as_ref(), 0)?
        .iter()
        .filter(|event| event.kind == CRITIC_REWORK_EVENT)
        .count();
    let plan = critic_rework::plan(&ReworkRequest {
        workflow_id,
        steps: &steps,
        gate: &gate,
        review: &review,
        prior_reworks,
    })
    .map_err(|e| e.to_string())?;

    step_control::redirect_gate(ctx, step_execution_id, &plan.feedback)
        .await
        .map_err(|e| e.to_string())?;
    let payload = serde_json::json!({
        "step_execution_id": step_execution_id.as_ref(),
        "verdict": plan.verdict.as_str(),
    })
    .to_string();
    ctx.run_events
        .append(
            feature.id.as_ref(),
            CRITIC_REWORK_EVENT,
            Some(&payload),
            crate::paths::now_ms(),
        )
        .map_err(|e| {
            format!(
                "the run was sent back for rework, but that could not be recorded \
                 against its limit: {e}"
            )
        })?;
    let reworks_used = prior_reworks + 1;
    Ok(CriticRework {
        feature_id: feature.id,
        verdict: plan.verdict.as_str(),
        reworks_used,
        reworks_remaining: MAX_CLIENT_REWORKS.saturating_sub(reworks_used),
    })
}

fn pinned_steps(ctx: &AppContext, feature: &Feature) -> Result<Vec<StepConfig>, String> {
    let version_id = feature
        .workflow_version_id
        .as_ref()
        .ok_or_else(|| "this run is not pinned to a workflow version".to_string())?;
    let version = ctx
        .workflows
        .version_get(version_id)?
        .ok_or_else(|| "this run's workflow version no longer exists".to_string())?;
    serde_json::from_str(&version.steps_json)
        .map_err(|e| format!("this run's workflow version could not be read: {e}"))
}

fn critic_review(
    ctx: &AppContext,
    feature_id: &FeatureId,
    keys: &[String],
) -> Result<String, String> {
    for key in keys {
        let found = ctx
            .artifact_store
            .list_for_step(feature_id.as_ref(), key)?
            .into_iter()
            .find(|reference| {
                std::path::Path::new(reference)
                    .file_stem()
                    .is_some_and(|stem| stem == CRITIC_ARTIFACT)
            });
        if let Some(reference) = found {
            return ctx.artifact_store.get(&reference);
        }
    }
    Err("the critic review for this run is not available".to_string())
}

#[cfg(test)]
#[path = "../../../tests/application/agent_surface_writes.rs"]
mod tests;
