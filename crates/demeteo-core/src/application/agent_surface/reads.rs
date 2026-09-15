//! Read operations for the `agent_surface` seam (module root docs). Every
//! function delegates to [`RunView`](crate::application::run_view::RunView)
//! or a repository port — none re-derive state those already expose.

use serde::Serialize;

use crate::application::tickets;
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId, StepExecutionId};
use crate::domain::models::{Feature, GateDecision, Project, StepAttempt, StepExecution};
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

/// A Discovery's tickets and derived board.
pub fn get_discovery_board(
    ctx: &AppContext,
    discovery_id: &DiscoveryId,
) -> Result<tickets::DiscoveryBoard, String> {
    tickets::board(ctx, discovery_id)
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

#[cfg(test)]
#[path = "../../../tests/application/agent_surface.rs"]
mod tests;
