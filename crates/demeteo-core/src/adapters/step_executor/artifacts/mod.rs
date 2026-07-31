pub(crate) mod add_exclusions;
pub(crate) mod attached;
pub(crate) mod declared;
pub(crate) mod materialize;
pub(crate) mod snapshot;

// Thin shims: `steps/sequence` and the planner name these seven through this
// module, and `steps/sequence` is not this refactor's workload.
pub(crate) use crate::adapters::step_executor::steps::agent::gate_decision::get_latest_gate_decision;
pub(crate) use crate::domain::artifact_contract::inject_artifact_contract;
pub(crate) use crate::domain::step_boundary::inject_operating_boundary;
pub(crate) use materialize::materialize_external_artifact_paths;
pub(crate) use materialize::materialize_user_attachments_to_worktree;

pub(crate) use attached::resolve_attached_artifacts;
pub(crate) use attached::resolve_attached_user_attachments;
pub(crate) use declared::commit_worktree_changes;
pub(crate) use declared::compute_git_diff;
pub(crate) use declared::read_worktree_file;
pub(crate) use declared::resolve_declared_artifacts;
pub(crate) use declared::{note_undelivered_artifacts, MissingArtifact};
pub use snapshot::WorktreeSnapshot;
