//! Merge executor port — the **feature ↔ upstream sync** flow.
//!
//! Wraps `git merge` of `origin/<base>` into a feature branch with
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
use crate::domain::models::{FeatureDrift, UpstreamSyncFailure, UpstreamSyncOutcome};
use async_trait::async_trait;

#[async_trait]
pub trait MergeExecutor: Send + Sync {
    /// Sync a feature branch with the latest `origin/<base_branch>`, where
    /// the base is the one
    /// [`diff_base::resolve`](crate::domain::diff_base::resolve) gives — the
    /// project's default branch only for a run that started there.
    ///
    /// - `Ok(UpstreamSyncOutcome)` when the feature branch was
    ///   fast-forwarded or a merge commit was created cleanly. The
    ///   `changed` flag is `false` when there was nothing to pull.
    /// - `Err(UpstreamSyncFailure::Conflict)` when the merge ran and left
    ///   unmerged paths. The `ConflictReport` embedded inside carries the
    ///   `ConflictFile` list the resolution agent and the UI render — possibly
    ///   an empty one, when the porcelain read that fills it failed.
    /// - `Err(UpstreamSyncFailure::Blocked)` for every other way this can
    ///   fail, most of which never issued a `git merge` at all. There is no
    ///   report and no file list: nothing is known to be conflicted, so the
    ///   resolution agent must not be offered one.
    ///
    /// Which of the two it is may never be inferred from the payload; only the
    /// variant answers it. [`crate::domain::sync_failure`] carries why, and is
    /// the one place that maps either to a view or to a workflow decision.
    #[allow(clippy::result_large_err)]
    async fn sync_feature_with_upstream(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        base_branch: &str,
    ) -> Result<UpstreamSyncOutcome, UpstreamSyncFailure>;

    /// How far this feature's branch has drifted from the base it will merge
    /// into, having merged nothing.
    ///
    /// `refresh` decides whether `origin/<base>` is fetched first: without it
    /// the answer is as of whenever that ref last moved, with it the answer is
    /// current and costs a network round trip. Either way the fetch's outcome
    /// is reported rather than enforced — this is the one caller that must
    /// **not** copy [`sync_feature_with_upstream`](Self::sync_feature_with_upstream)'s
    /// hard failure on a bad fetch, because a poll that errors costs the user
    /// the whole signal where a poll that says "as of your last sync" still
    /// tells them something true.
    ///
    /// Here rather than on
    /// [`WorktreeOpsPort`](crate::ports::worktree_ops::WorktreeOpsPort) because
    /// the question is feature-scoped and that port is repo-scoped: only this
    /// port already turns a feature id into a machine and a repo directory.
    async fn feature_drift(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        base_branch: &str,
        refresh: bool,
    ) -> Result<FeatureDrift, String>;

    /// The feature's live sync as the working tree says it stands, or `None`
    /// when it has never synced.
    ///
    /// Reconciled before it answers, on the terms
    /// [`SyncSessionPort`](crate::ports::sync_session::SyncSessionPort) sets: a
    /// caller acting on the stored status alone would resolve a conflict whose
    /// worktree a later attempt has already removed.
    ///
    /// Here for the reason `record_sync_resolution` is: this implementation owns
    /// every write to that row, and the two callers that need to read it hold a
    /// `MergeExecutor` already.
    async fn sync_session(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<crate::ports::sync_session::SyncSession>, String>;

    /// Move the sync session on to what a resolution turn is doing, or has done.
    ///
    /// Here rather than on a session port of its own because this
    /// implementation already owns every write to that row, and the callers that
    /// need it — the `sync` workflow step and the "Resolve with agent" button —
    /// already hold a `MergeExecutor`. Handing the step executor a second port
    /// to reach the same table would mean widening a constructor AGENTS.md §3
    /// already names as a review trigger.
    ///
    /// A session that never opened is not an error: the resolver can be reached
    /// from a conflict this process did not record.
    async fn record_sync_resolution(
        &self,
        feature_id: &FeatureId,
        resolution: &crate::domain::sync_session::SyncResolution,
    ) -> Result<(), String>;
}
