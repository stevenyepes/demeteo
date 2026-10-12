//! Read operations for the `agent_surface` seam (module root docs). Every
//! function delegates to [`RunView`](crate::application::run_view::RunView)
//! or a repository port — none re-derive state those already expose.

use serde::Serialize;

use crate::application::run_view::FailureExplanation;
use crate::application::tickets;
use crate::domain::ids::{DiscoveryId, FeatureId, MachineId, ProjectId, StepExecutionId};
use crate::domain::models::{
    Discovery, DiscoveryStatus, Feature, GateDecision, Machine, Project, StepAttempt, StepExecution,
};
use crate::domain::run_placement::{placement_for, RunPlacement};
use crate::ports::run_events::RunEvent;
use crate::state::AppContext;

/// A feature plus its full step-execution list — the shape [`get_feature`]
/// hands an external caller in one round trip instead of two.
#[derive(Debug, Clone, Serialize)]
pub struct FeatureWithSteps {
    pub feature: Feature,
    pub steps: Vec<StepExecution>,
}

/// One project's one pending gate, named — [`list_pending_gates`] is
/// cross-project, so the bare [`GateDecision`] (which carries no project or
/// feature title) isn't enough for a caller to say what it's looking at.
#[derive(Debug, Clone, Serialize)]
pub struct PendingGate {
    pub project_id: ProjectId,
    pub feature_id: FeatureId,
    pub feature_title: String,
    pub decision: GateDecision,
}

/// A Discovery as a caller choosing one reads it. Narrower than the row on
/// purpose: the interview's session id, worktree path, attachments and spend
/// are the app's own bookkeeping, and nothing here needs them to name a
/// Discovery and say where its tickets integrate.
#[derive(Debug, Clone, Serialize)]
pub struct DiscoverySummary {
    pub id: DiscoveryId,
    pub project_id: ProjectId,
    pub title: String,
    pub status: DiscoveryStatus,
    /// `None` is the project's default branch, as on the row.
    pub base_branch: Option<String>,
    pub updated_at: i64,
}

impl From<Discovery> for DiscoverySummary {
    fn from(d: Discovery) -> Self {
        Self {
            id: d.id,
            project_id: d.project_id,
            title: d.title,
            status: d.status,
            base_branch: d.base_branch,
            updated_at: d.updated_at,
        }
    }
}

/// A registered machine as a caller placing a run reads it: the id a
/// `machine_id` argument takes, and whether naming it detaches the run.
///
/// The row's host, user, key path and webhook URL are deliberately absent. A
/// `read` grant spans every project, and none of them helps choose a
/// placement — a webhook URL is, besides, a credential in all but name.
#[derive(Debug, Clone, Serialize)]
pub struct MachineSummary {
    pub id: MachineId,
    pub name: String,
    /// What a launch naming this id resolves to. `local` is this desktop,
    /// which a detached launch refuses.
    pub placement: RunPlacement,
}

impl From<Machine> for MachineSummary {
    fn from(m: Machine) -> Self {
        let placement = if m.auth_type == "local" {
            RunPlacement::Local
        } else {
            placement_for(&m.id)
        };
        Self {
            id: m.id,
            name: m.name,
            placement,
        }
    }
}

/// Every workspace project.
pub fn list_projects(ctx: &AppContext) -> Result<Vec<Project>, String> {
    ctx.projects.get_projects()
}

/// Active features, either for one project or — when `project_id` is
/// `None` — the union across every project.
///
/// The `None` case composes `get_projects` with one `get_active` call per
/// project (N+1) rather than a dedicated cross-project query: no existing
/// port method answers "every active feature regardless of project", and the
/// workspace project count this fans out over is small enough that adding
/// one is a later decision, not a wiring one.
pub fn list_features(
    ctx: &AppContext,
    project_id: Option<&ProjectId>,
) -> Result<Vec<Feature>, String> {
    match project_id {
        Some(id) => ctx.features.get_active(id),
        None => {
            let mut features = Vec::new();
            for project in ctx.projects.get_projects()? {
                features.extend(ctx.features.get_active(&project.id)?);
            }
            Ok(features)
        }
    }
}

/// A feature and its steps in one round trip, both read through
/// [`ctx.run_view`](crate::application::run_view::RunView) — the same seam
/// the UI renders a run from. `None` when the feature does not exist.
pub fn get_feature(
    ctx: &AppContext,
    feature_id: &FeatureId,
) -> Result<Option<FeatureWithSteps>, String> {
    let Some(feature) = ctx.run_view.feature(feature_id)? else {
        return Ok(None);
    };
    let steps = ctx.run_view.steps(feature_id)?;
    Ok(Some(FeatureWithSteps { feature, steps }))
}

/// Per-attempt history for a step, ordered by attempt number.
pub fn list_step_attempts(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
) -> Result<Vec<StepAttempt>, String> {
    ctx.run_view.step_attempts(step_execution_id)
}

/// Every gate awaiting a decision across a project's — or, when
/// `project_id` is `None`, every project's — active features.
///
/// Composes [`list_features`] with the sync `GateRepository` port's
/// `pending_for_feature` directly, one call per feature, rather than the
/// async `StepExecutor::gate_pending_for_run` presenter wrapper: a read
/// needs no executor dependency.
pub fn list_pending_gates(
    ctx: &AppContext,
    project_id: Option<&ProjectId>,
) -> Result<Vec<PendingGate>, String> {
    let mut pending = Vec::new();
    for feature in list_features(ctx, project_id)? {
        if let Some(decision) = ctx.gates.pending_for_feature(&feature.id)? {
            pending.push(PendingGate {
                project_id: feature.project_id.clone(),
                feature_id: feature.id.clone(),
                feature_title: feature.title.clone(),
                decision,
            });
        }
    }
    Ok(pending)
}

/// Discoveries, most recently touched first within each project — for one
/// project, or across all of them on [`list_features`]'s terms. Closed ones
/// are included, as the port includes them.
pub fn list_discoveries(
    ctx: &AppContext,
    project_id: Option<&ProjectId>,
) -> Result<Vec<DiscoverySummary>, String> {
    let project_ids = match project_id {
        Some(id) => vec![id.clone()],
        None => ctx
            .projects
            .get_projects()?
            .into_iter()
            .map(|p| p.id)
            .collect(),
    };
    let mut discoveries = Vec::new();
    for id in &project_ids {
        discoveries.extend(
            ctx.discoveries
                .list_for_project(id)?
                .into_iter()
                .map(|row| DiscoverySummary::from(row.discovery)),
        );
    }
    Ok(discoveries)
}

/// A Discovery's tickets and derived board.
pub fn get_discovery_board(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
) -> Result<tickets::DiscoveryBoard, String> {
    tickets::board(ctx, discovery_id)
}

/// [`get_discovery_board`] after reading each unsettled pull request from the
/// forge ([`tickets::refresh`]).
pub async fn refresh_discovery_prs(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
) -> Result<tickets::refresh::RefreshedBoard, String> {
    tickets::refresh::refresh_pr_states(ctx, discovery_id).await
}

/// Every registered machine.
///
/// Rows only: whether a machine's runner is installed, reachable and this
/// build's version takes an SSH round trip per machine, which a launch makes
/// for the one machine it names and refuses on. A list that probed them all
/// would be a `read` that opens connections.
pub fn list_machines(ctx: &AppContext) -> Result<Vec<MachineSummary>, String> {
    Ok(ctx
        .machines
        .get_machines()?
        .into_iter()
        .map(MachineSummary::from)
        .collect())
}

/// Durable events for a feature whose offsets are greater than
/// `from_offset`, in ascending offset order.
pub fn run_events_since(
    ctx: &AppContext,
    feature_id: &FeatureId,
    from_offset: i64,
) -> Result<Vec<RunEvent>, String> {
    ctx.run_view.run_events_since(feature_id, from_offset)
}

/// Why a step failed, for a coding agent asking about its own failed run.
pub fn get_failure_verdict(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
) -> Result<FailureExplanation, String> {
    ctx.run_view.explain_step_failure(step_execution_id)
}

#[cfg(test)]
#[path = "../../../tests/application/agent_surface.rs"]
mod tests;
