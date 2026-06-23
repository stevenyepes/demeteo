pub(crate) mod attached;
pub(crate) mod declared;
pub(crate) mod snapshot;

pub(crate) use attached::get_latest_gate_decision;
pub(crate) use attached::inject_artifact_contract;
pub(crate) use attached::resolve_attached_artifacts;
pub(crate) use declared::commit_worktree_changes;
pub(crate) use declared::compute_git_diff;
pub(crate) use declared::read_worktree_file;
pub(crate) use declared::resolve_declared_artifacts;
pub use snapshot::WorktreeSnapshot;
