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
use crate::domain::models::{
    FeatureDivergence, FeatureDrift, UpstreamSyncFailure, UpstreamSyncOutcome,
};
use crate::domain::upstream_feature::DivergenceReconcile;
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
    ///
    /// `gate` is what the merged tree must prove before the merge reaches
    /// origin ([`MergeGate`](crate::ports::worktree_ops::MergeGate)). It comes
    /// from the caller rather than from a project row read here, because both
    /// callers have already resolved the same
    /// [`ProjectSettings`](crate::domain::models::ProjectSettings) to work out
    /// which base to sync from — and a second lookup is a second chance for the
    /// button and the workflow node to gate on different commands.
    #[allow(clippy::result_large_err)]
    async fn sync_feature_with_upstream(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        base_branch: &str,
        gate: crate::ports::worktree_ops::MergeGate<'_>,
    ) -> Result<UpstreamSyncOutcome, UpstreamSyncFailure>;

    /// Reconcile the feature branch with `origin/<feature>` the way a person
    /// chose, and then sync it — the press behind a sync that stopped on a
    /// divergence it may not settle alone.
    ///
    /// The same call as
    /// [`sync_feature_with_upstream`](Self::sync_feature_with_upstream) with the
    /// divergence answered, so a reconcile that conflicts is an ordinary
    /// conflicted session and everything downstream of it is unchanged. That is
    /// also why the answer is the session and not an outcome: the verdict the
    /// user acts on next is the row, and a reconcile has more ways to end than
    /// the press has arms.
    ///
    /// `Err` is reserved for the two stages that stop before the row exists
    /// ([`SyncBlockedStage::precedes_the_session`](crate::domain::sync_failure::SyncBlockedStage::precedes_the_session));
    /// answering with the row there would hand back the *previous* sync's
    /// verdict as this press's result.
    async fn reconcile_feature_with_origin(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        base_branch: &str,
        gate: crate::ports::worktree_ops::MergeGate<'_>,
        reconcile: DivergenceReconcile,
    ) -> Result<Option<crate::ports::sync_session::SyncSessionView>, String>;

    /// What this branch and `origin/<feature>` each hold that the other does
    /// not, and which of git's moves that leaves open — `None` when they do not
    /// disagree, or when the refs could not be read.
    ///
    /// A read, for a pane that has to decide which presses to offer *before*
    /// anyone makes one. It re-measures rather than reading the counts the
    /// blocked row recorded ([`crate::domain::upstream_feature`]).
    async fn feature_divergence(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
    ) -> Result<Option<FeatureDivergence>, String>;

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
