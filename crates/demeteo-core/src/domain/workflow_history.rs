//! Version numbering and identity for a workflow's history.
//!
//! Every path that produces a version — an edit from the builder, a revert to
//! the bundled starter, a restore from history, the first-launch seed — asks
//! the same two questions here, so "saving is an append, never an edit" stays
//! one fact instead of four copies of the same arithmetic.
//!
//! Decisions only: the append itself is a repository call, and the reason that
//! split is worth making is on [`crate::domain`].

use crate::domain::ids::{WorkflowId, WorkflowVersionId};
use crate::domain::models::WorkflowVersion;

/// One past the highest version that exists. Numbering never reuses a value,
/// so a version id derived from it is unique for the life of the workflow.
pub fn next_version_number(existing: &[WorkflowVersion]) -> u32 {
    existing.iter().map(|v| v.version).max().unwrap_or(0) + 1
}

/// The id that version `version` of `workflow_id` carries.
///
/// Derived from the pair rather than minted at random, which is what makes
/// [`next_version_number`]'s never-reuse rule the uniqueness guarantee — and
/// what makes a version id guessable, hence [`ensure_owned_by`].
pub fn version_id(workflow_id: &WorkflowId, version: u32) -> WorkflowVersionId {
    WorkflowVersionId::from(format!("{}-v{}", workflow_id.as_str(), version))
}

/// Prove a loaded version row belongs to the workflow the caller named.
///
/// Version ids are guessable by construction ([`version_id`]), so the pairing
/// is checked rather than assumed — a mismatched pair would otherwise let one
/// workflow's history be restored onto another.
pub fn ensure_owned_by(version: &WorkflowVersion, workflow_id: &WorkflowId) -> Result<(), String> {
    if &version.workflow_id != workflow_id {
        return Err(format!(
            "Version {} belongs to workflow {}, not {workflow_id}.",
            version.id, version.workflow_id
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../tests/domain/workflow_history.rs"]
mod tests;
