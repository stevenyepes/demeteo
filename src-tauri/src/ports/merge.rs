//! Merge executor port (Phase R6).
//!
//! Wraps the existing [`crate::adapters::worktree::git_ops::GitOpsHelper`]
//! `merge_subtask` call with structured conflict detection and a
//! typed result. Implementations record the outcome in the
//! `subtask_merges` table so the audit trail survives step teardown.

use crate::domain::ids::FeatureId;
use crate::domain::models::{ConflictReport, MergeOutcome};

pub trait MergeExecutor: Send + Sync {
    /// Merge `source_branch` into `target_branch` (the feature branch).
    ///
    /// - `Ok(MergeOutcome)` on a clean merge (caller can mark the
    ///   subtask complete).
    /// - `Err(ConflictReport)` if git reports a conflict. The caller
    ///   is responsible for routing this through the project's
    ///   `conflict_policy` (cascade).
    fn merge_subtask_into_feature(
        &self,
        feature_id: &FeatureId,
        source_branch: &str,
        target_branch: &str,
        subtask_run_id: &str,
    ) -> Result<MergeOutcome, ConflictReport>;

    /// Skip the merge entirely (user picked "Skip" in the cascade).
    /// Recorded as a `subtask_merges` row with `status = 'skipped'`.
    fn skip_merge(&self, subtask_run_id: &str, reason: &str) -> Result<(), String>;

    /// Abort any in-progress git merge state on the target branch
    /// (e.g. after a hard failure mid-merge). Does not record a
    /// `subtask_merges` row — the existing pending row stays
    /// pending until the next attempt resolves it.
    fn abort_in_progress(&self, target_branch: &str) -> Result<(), String>;
}