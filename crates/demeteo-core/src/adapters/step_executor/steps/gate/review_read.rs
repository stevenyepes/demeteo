//! Reading the review a ship gate may be approved on.

use crate::domain::gate::review_contract::{review_step_for_gate, standing, ReviewStanding};
use crate::domain::ids::StepId;
use crate::domain::models::{StepConfig, StepExecution};
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
    step_execs: &[StepExecution],
    gate_step_id: &StepId,
) -> ReviewStanding {
    let Some(review_step) = steps
        .iter()
        .position(|step| &step.id == gate_step_id)
        .and_then(|gate_idx| review_step_for_gate(steps, gate_idx))
    else {
        return ReviewStanding::Unreadable("no review step covers this gate");
    };
    let completed = step_execs
        .iter()
        .any(|exec| exec.step_id == review_step.id && exec.status == "completed");
    if !completed {
        return ReviewStanding::Unreadable("the review step has not completed");
    }
    let Some(name) = review_step.review_artifact.as_deref() else {
        return ReviewStanding::Unreadable("no review step covers this gate");
    };
    // The last reference, not the first: a rework cycle re-runs the review,
    // and the store lists a step's references in insertion order.
    let review = artifacts
        .list_for_step(feature_id, review_step.id.as_ref())
        .ok()
        .and_then(|references| {
            references.into_iter().rev().find(|reference| {
                std::path::Path::new(reference)
                    .file_stem()
                    .is_some_and(|stem| stem == name)
            })
        })
        .and_then(|reference| artifacts.get(&reference).ok());
    match review {
        Some(review) => standing(&review),
        None => ReviewStanding::Unreadable("the review artifact is not in the store"),
    }
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/gate_review_read.rs"]
mod tests;
