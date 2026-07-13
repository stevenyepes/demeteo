//! Merge executor port — the **feature ↔ upstream sync** flow.
//!
//! Wraps `git merge` of `origin/<default>` into a feature branch with
//! structured conflict detection and a `feature_syncs` audit trail. Serves
//! the "Sync with main" button, the `sync` workflow step, and the
//! "Resolve with agent" recovery flow.
//!
//! This port used to also own the subtask→feature merge (`
//! merge_subtask_into_feature`, the R6 cascade). That half was never called:
//! the steps that merge task branches back — `steps::agent` and
//! `steps::sequence` — do it inline via `GitOpsHelper::merge_subtask` and
//! resolve conflicts with `steps::conflict_pass`, using the worktree and
//! session they already hold. It was deleted rather than maintained as
//! fiction; see `docs/DECISIONS.md` decision 20's history.
//!
//! **All methods are async.** Tauri v2 supports async commands natively.

use crate::domain::ids::FeatureId;
use crate::domain::models::{UpstreamSyncFailure, UpstreamSyncOutcome};
use async_trait::async_trait;

#[async_trait]
pub trait MergeExecutor: Send + Sync {
    /// Sync a feature branch with the latest `origin/<default_branch>`.
    ///
    /// - `Ok(UpstreamSyncOutcome)` when the feature branch was
    ///   fast-forwarded or a merge commit was created cleanly. The
    ///   `changed` flag is `false` when there was nothing to pull.
    /// - `Err(UpstreamSyncFailure)` when the merge produced
    ///   conflicts. The `ConflictReport` embedded inside carries
    ///   the `ConflictFile` list the resolution agent and the UI
    ///   render.
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
