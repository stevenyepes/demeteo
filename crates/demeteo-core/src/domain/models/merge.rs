use crate::domain::ids::FeatureId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FeatureSync {
    pub id: String,
    pub feature_id: FeatureId,
    pub feature_branch: String,
    pub default_branch: String,
    /// pending | ok | conflict | blocked | skipped | aborted
    pub status: String,
    pub merge_commit_sha: Option<String>,
    /// JSON-encoded [`ConflictReport`] when `status == "conflict"`, and the
    /// JSON-encoded [`UpstreamSyncFailure`] when `status == "blocked"` — the
    /// column is the audit trail's one free-form slot and both classes need a
    /// reason kept.
    pub conflict_report: Option<String>,
    pub resolution_attempts: i32,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContext {
    pub compute_type: String,
    pub remote_host: Option<String>,
    pub project_id: String,
    pub repo_path: String,
}

/// One file in a conflict set. Path is repo-relative.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictFile {
    pub path: String,
    /// Short one-line summary ("both modified", "deleted by us",
    /// "deleted by them", "added by both", …).
    pub kind: String,
}

/// `git merge` / `git rebase` returned this — the merge executor
/// surfaces it so the resolution agent and the UI have structured
/// data to work with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConflictReport {
    pub source_branch: String,
    pub target_branch: String,
    pub files: Vec<ConflictFile>,
    /// Raw stderr from the failing git invocation. Useful for the
    /// manual-resolution UI ("look at the actual git error").
    pub raw_error: String,
    /// Detected at: ms-since-epoch. Helps the UI render "X minutes ago".
    pub detected_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree_path: Option<String>,
}

/// Result of `MergeExecutor::sync_feature_with_upstream` on a clean
/// merge. The caller is expected to record the audit row and let
/// the workflow execution continue.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpstreamSyncOutcome {
    /// SHA of the merge commit (empty when there was nothing to merge).
    pub merge_commit_sha: String,
    /// `false` when `origin/<default>` had no new commits since the
    /// last sync.
    pub changed: bool,
    /// The default branch we synced against.
    pub default_branch: String,
}

/// How stale a feature branch is against the base it will merge into, answered
/// without merging anything.
///
/// `fetched` is part of the answer rather than an implementation detail: the
/// counts are taken from `refs/remotes/origin/<base>`, so a query that skipped
/// or failed its fetch is reporting how things stood at some unnamed earlier
/// moment. A number presented as current when it is not is worse than no
/// number, because the user acts on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeatureDrift {
    pub divergence: crate::ports::worktree_ops::BranchDivergence,
    /// The ref the counts were taken against.
    pub base_ref: String,
    pub fetched: bool,
    pub checked_at: i64,
}

/// Result of a failed upstream sync, mirroring
/// [`SyncFailure`](crate::ports::worktree_ops::SyncFailure) across the merge
/// executor. In-memory only — [`ConflictReport`] is the persisted half.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum UpstreamSyncFailure {
    Conflict {
        report: ConflictReport,
        /// Path to the sync worktree where the conflict lives (if one was
        /// provisioned).
        worktree_path: Option<String>,
    },
    Blocked {
        stage: crate::domain::sync_failure::SyncBlockedStage,
        raw_error: String,
    },
}
