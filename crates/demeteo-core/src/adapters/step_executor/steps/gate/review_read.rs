//! Reading the review a ship gate may be approved on.

use std::path::Path;

use crate::domain::gate::review_contract::{review_for_gate, standing, ReviewStanding};
use crate::domain::ids::StepId;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::workflow_graph::WorkflowGraph;
use crate::ports::artifact_store::ArtifactStore;

/// The standing of the review covering the gate `gate_step_id`, read from the
/// artifact store.
///
/// The store rather than the worktree, because the gate may be reached after
/// a restart or on another transport and the stored copy is the one that is
/// there either way. Anything short of a completed review step with a
/// readable artifact is [`ReviewStanding::Unreadable`], which asks a person.
pub(crate) fn review_standing(
    artifacts: &dyn ArtifactStore,
    feature_id: &str,
    steps: &[StepConfig],
    graph: &WorkflowGraph,
    step_execs: &[StepExecution],
    gate_step_id: &StepId,
) -> ReviewStanding {
    let Some(review) = review_for_gate(steps, graph, gate_step_id) else {
        return ReviewStanding::Unreadable("no review step covers this gate");
    };
    let completed = step_execs
        .iter()
        .any(|exec| exec.step_id == review.step.id && exec.status == "completed");
    if !completed {
        return ReviewStanding::Unreadable("the review step has not completed");
    }
    // By the declared path's file name, never by the declaration's name: a
    // body captured from the worktree is stored under the name of the file it
    // was read from, and most workflows give the two different names
    // (`critic` written to `artifacts/s-critic.md`).
    let stored_as = Path::new(review.path).file_name();
    let body = artifacts
        .list_for_step(feature_id, review.step.id.as_ref())
        .ok()
        .and_then(|references| {
            references.into_iter().find(|reference| {
                stored_as.is_some() && Path::new(reference).file_name() == stored_as
            })
        })
        .and_then(|reference| artifacts.get(&reference).ok());
    match body {
        Some(body) => standing(&body),
        None => ReviewStanding::Unreadable("the review artifact is not in the store"),
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/gate_review_read.rs"]
mod tests;
