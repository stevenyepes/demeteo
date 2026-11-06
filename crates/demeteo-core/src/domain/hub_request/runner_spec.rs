//! The run a `demeteo-runner` starts for a Hub `start_feature` (docs/HUB.md
//! §6.1 *On an instance*): the signed [`RunnerStart`] stands in for what a
//! desktop would find in its own database, and this maps it onto the
//! [`RunSpec`] a desktop's detached launch would have submitted.
//!
//! Everything a runner's start reads is checked here, before any attachment
//! is fetched, because a fetch deletes the Hub's copy: [`runner_start_refusal`]
//! is what the runner's `resolve` answers, and [`runner_run_spec`] applies the
//! same checks again so no spec is built from a start that skipped them.

use demeteo_hub_protocol::{RefusalReason, RunnerStart, StartFeature};

use super::{effort_level, step_override};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::models::{LaunchedBy, StepConfig};
use crate::domain::run_spec::{RunBudget, RunOrigin, RunSpec, RunSpecAttachment, RunSpecProvider};

/// The provider kinds a runner can clone from; `RunSpecProvider.kind` names
/// one of them.
const PROVIDER_KINDS: [&str; 2] = ["github", "gitlab"];

/// Why a runner would not start `start`, or `None` when it would.
///
/// Every refusal is `MalformedBody`: the Hub resolved and signed what is
/// here, so a start that fails these is the Hub's mistake rather than a
/// lookup this instance could answer `unknown_*` to. That includes a start
/// with no `runner` at all, which only a Hub that predates runner dispatch
/// sends, and one whose `workflow_id` disagrees with the workflow it signed.
pub fn runner_start_refusal(start: &StartFeature) -> Option<RefusalReason> {
    match start.runner.as_deref() {
        None => Some(RefusalReason::MalformedBody),
        Some(runner) => checked_workflow(start, runner).err(),
    }
}

/// The spec a runner submits for `start`, under the Feature id it minted.
///
/// The cap is both the per-turn `max_budget_usd`, as a desktop launch reads
/// `max_budget_cents`, and the run's hard [`RunBudget`], which on a runner
/// also carries the wall clock a desktop cannot honour. The run is
/// unattended, as every detached run is; it starts from the default branch,
/// since a Hub start names no origin; and it carries no project settings, so
/// the runner's detected defaults apply.
pub fn runner_run_spec(
    start: &StartFeature,
    runner: &RunnerStart,
    feature_id: &str,
    attachments: Vec<RunSpecAttachment>,
) -> Result<RunSpec, RefusalReason> {
    let steps = checked_workflow(start, runner)?;
    let cost = start.max_budget_cents.map(|cents| cents as f64 / 100.0);
    let budget = (cost.is_some() || start.max_wall_clock_secs.is_some()).then_some(RunBudget {
        max_cost_usd: cost,
        max_wall_clock_secs: start.max_wall_clock_secs,
    });
    Ok(RunSpec {
        feature_id: Some(feature_id.to_owned()),
        title: start.title.clone(),
        description: start.description.clone(),
        provider: RunSpecProvider {
            kind: runner.repository.provider_kind.clone(),
            host: runner.repository.host.clone(),
        },
        repo_path: runner.repository.full_name.clone(),
        workflow_json: serde_json::json!({
            "name": runner.workflow.name,
            "description": "",
            "steps": steps,
        }),
        agent_kind: start.agent_kind.clone(),
        model: start.model.clone(),
        effort: start.effort.map(effort_level),
        loop_iterations: None,
        max_budget_usd: cost,
        step_overrides: start.step_overrides.iter().map(step_override).collect(),
        commit_artifacts: None,
        attachments,
        unattended: true,
        budget,
        project_settings: None,
        origin: Some(RunOrigin::Supported(FeatureOrigin::DefaultBranch)),
        diff_base_branch: None,
        launched_by: LaunchedBy::Hub,
    })
}

/// The workflow's steps, parsed as the runner's ingestion will parse them,
/// once the repository is one a runner can clone. An empty step list is
/// refused here too: ingestion refuses it, and by then the attachments are
/// gone.
fn checked_workflow(
    start: &StartFeature,
    runner: &RunnerStart,
) -> Result<serde_json::Value, RefusalReason> {
    let repo = &runner.repository;
    let repository_ok = PROVIDER_KINDS.contains(&repo.provider_kind.as_str())
        && !repo.host.is_empty()
        && !repo.full_name.is_empty();
    if !repository_ok || runner.workflow.workflow_id != start.workflow_id {
        return Err(RefusalReason::MalformedBody);
    }
    let steps: serde_json::Value = serde_json::from_str(&runner.workflow.steps_json)
        .map_err(|_| RefusalReason::MalformedBody)?;
    match serde_json::from_value::<Vec<StepConfig>>(steps.clone()) {
        Ok(parsed) if !parsed.is_empty() => Ok(steps),
        _ => Err(RefusalReason::MalformedBody),
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/hub_request/runner_spec.rs"]
mod tests;
