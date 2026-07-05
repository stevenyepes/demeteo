//! Merge executor port.
//!
//! Wraps `git merge` for both subtask→feature and feature→upstream
//! flows with structured conflict detection and an audit trail.
//!
//! **All methods are async.** Tauri v2 supports async commands natively.

use crate::domain::ids::FeatureId;
use crate::domain::models::{
    ConflictReport, MergeOutcome, UpstreamSyncFailure, UpstreamSyncOutcome,
};
use async_trait::async_trait;

#[async_trait]
pub trait MergeExecutor: Send + Sync {
    /// Merge `source_branch` into `target_branch` (the feature branch).
    ///
    /// - `Ok(MergeOutcome)` on a clean merge (caller can mark the
    ///   subtask complete).
    /// - `Err(ConflictReport)` if git reports a conflict. The caller
    ///   is responsible for routing this through the project's
    ///   `conflict_policy` (cascade).
    #[allow(clippy::result_large_err)]
    async fn merge_subtask_into_feature(
        &self,
        feature_id: &FeatureId,
        source_branch: &str,
        target_branch: &str,
        subtask_run_id: &str,
    ) -> Result<MergeOutcome, ConflictReport>;

    /// Skip the merge entirely (user picked "Skip" in the cascade).
    /// Recorded as a `subtask_merges` row with `status = 'skipped'`.
    async fn skip_merge(&self, subtask_run_id: &str, reason: &str) -> Result<(), String>;

    /// Abort any in-progress git merge state on the target branch
    /// (e.g. after a hard failure mid-merge). Does not record a
    /// `subtask_merges` row — the existing pending row stays
    /// pending until the next attempt resolves it.
    async fn abort_in_progress(&self, target_branch: &str) -> Result<(), String>;

    /// Sync a feature branch with the latest `origin/<default_branch>`.
    ///
    /// This is the **upstream** counterpart of `merge_subtask_into_feature`:
    /// the source is `origin/<default>` and the target is the user's
    /// feature branch. The result has the same shape as the subtask
    /// merge result so the same conflict-resolver cascade can be
    /// reused.
    ///
    /// - `Ok(UpstreamSyncOutcome)` when the feature branch was
    ///   fast-forwarded or a merge commit was created cleanly. The
    ///   `changed` flag is `false` when there was nothing to pull.
    /// - `Err(UpstreamSyncFailure)` when the merge produced
    ///   conflicts. The `ConflictReport` embedded inside carries
    ///   the same `ConflictFile` list that the subtask merge
    ///   produces, so the resolver sees a uniform data shape.
    #[allow(clippy::result_large_err)]
    async fn sync_feature_with_upstream(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        default_branch: &str,
    ) -> Result<UpstreamSyncOutcome, UpstreamSyncFailure>;

    /// Retrieve the worktree path from the last sync conflict report.
    async fn get_last_sync_worktree_path(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<String>, String>;
}
