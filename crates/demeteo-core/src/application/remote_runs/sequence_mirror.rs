use crate::domain::ids::FeatureId;
use crate::domain::models::SequenceStateMirror;
use crate::ports::db::{FeatureRepository, SequenceResumeRepository};

/// One `sequence` node's resume state, read from whichever database holds
/// it — the runner's own when it answers `get_sequence_state`, the laptop's
/// mirror when it computes the `if_revision` it sends. One function for both
/// is what lets [`SequenceStateMirror::revision`] ever match: two hand-rolled
/// reads drift, and every drift is a full plan resent on every poll.
pub fn read_sequence_state_mirror(
    features: &dyn FeatureRepository,
    sequence_resume: &dyn SequenceResumeRepository,
    feature_id: &FeatureId,
    node_id: &str,
) -> Result<SequenceStateMirror, String> {
    let plan_json = sequence_resume.plan_cache_get(feature_id, node_id)?;
    let checkpoint = sequence_resume.sequence_checkpoint_get(feature_id, node_id)?;
    let steps = features.steps_for_feature(feature_id)?;
    let subtask_runs = match steps.iter().find(|s| s.step_id.as_str() == node_id) {
        Some(step) => features.subtask_runs_mirror_for_step(&step.id)?,
        None => Vec::new(),
    };
    Ok(SequenceStateMirror {
        plan_json,
        checkpoint,
        subtask_runs,
    })
}
