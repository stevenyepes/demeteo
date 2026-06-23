use crate::domain::ids::FeatureId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SubtaskMerge {
    pub id: String,
    pub subtask_run_id: String,
    pub feature_id: FeatureId,
    pub source_branch: String,
    pub target_branch: String,
    /// pending | ok | conflict | skipped | aborted
    pub status: String,
    pub merge_commit_sha: Option<String>,
    /// JSON-encoded [`ConflictReport`] when `status == "conflict"`.
    pub conflict_report: Option<String>,
    pub resolution_attempts: i32,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FeatureSync {
    pub id: String,
    pub feature_id: FeatureId,
    pub feature_branch: String,
    pub default_branch: String,
    /// pending | ok | conflict | skipped | aborted
    pub status: String,
    pub merge_commit_sha: Option<String>,
    /// JSON-encoded [`ConflictReport`] when `status == "conflict"`.
    pub conflict_report: Option<String>,
    pub resolution_attempts: i32,
    pub created_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorktreeContext {
    pub compute_type: String,
    pub remote_host: Option<String>,
    pub project_id: String,
    pub repo_path: String,
    pub worktree_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoContext {
    pub compute_type: String,
    pub remote_host: Option<String>,
    pub project_id: String,
    pub repo_path: String,
}

/// Result of a successful merge. `Ok` from [`MergeExecutor::merge_subtask_into_feature`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MergeOutcome {
    pub merge_commit_sha: String,
    pub target_branch: String,
    pub source_branch: String,
    /// True when the subtask was already an ancestor of the feature
    /// branch — nothing needed to be merged. The SHA is the feature
    /// branch tip at the time of the check.
    pub already_merged: bool,
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
/// surfaces it so the conflict resolver cascade has structured
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

/// Result of a failed upstream sync — the merge left the working
/// tree in a conflicted state. Same `ConflictReport` shape as the
/// subtask merge failure so the cascade has a uniform input.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UpstreamSyncFailure {
    pub report: ConflictReport,
    /// Path to the sync worktree where the conflict lives (if one was
    /// provisioned). `None` when the sync was aborted before a working
    /// tree was needed.
    pub worktree_path: Option<String>,
}

/// Per-project setting that controls how a merge conflict is
/// resolved. Mirrors the dropdown in `ProjectSettings`'s
/// "Conflict Resolution Policy" field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// Always surface a gate; never auto-merge.
    AlwaysGate,
    /// Try the auto-agent first; cascade to manual on failure.
    AutoAgent,
    /// Skip the auto-agent; immediately open the manual UI.
    AutoHuman,
}

impl ConflictPolicy {
    pub fn from_db(s: &str) -> Self {
        match s {
            "auto_agent" => ConflictPolicy::AutoAgent,
            "auto_human" => ConflictPolicy::AutoHuman,
            _ => ConflictPolicy::AlwaysGate,
        }
    }
}
