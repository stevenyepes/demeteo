//! Worktree operations port.
//!
//! Provides abstract access to Git worktree operations such as cloning,
//! provisioning worktrees, checking repository state, and syncing with upstream.

use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use async_trait::async_trait;

/// Result of a successful feature branch sync.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// SHA of the merge commit (empty when there was nothing to merge).
    pub merge_commit_sha: String,
    /// `false` when `origin/<default>` didn't exist or had no new
    /// commits since the last sync.
    pub changed: bool,
}

/// Result of a failed sync — the merge left the working tree in a
/// conflicted state. The caller is expected to spawn a resolution
/// agent or hand the files back to the user.
#[derive(Debug, Clone)]
pub struct SyncFailure {
    pub files: Vec<crate::domain::models::ConflictFile>,
    pub raw_error: String,
    /// Path to the sync worktree where the conflicted state lives.
    /// `None` when the sync was aborted before a worktree was created.
    pub worktree_path: Option<String>,
}

/// Result of a pre-merge check (no working tree touched).
#[derive(Debug, Clone, PartialEq)]
pub enum MergePreCheck {
    /// Subtask is already an ancestor of origin/feature_branch — skip.
    AlreadyMerged,
    /// Merge would be clean — proceed safely.
    CleanMerge,
    /// Merge would produce conflicts — use the resolver cascade.
    WouldConflict,
}

#[async_trait]
pub trait WorktreeOpsPort: Send + Sync {
    /// Check if the repository is dirty.
    async fn check_repo_dirty(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String>;

    /// Retrieve the HEAD branch name.
    async fn get_head_branch(&self, machine_id: Option<&str>, repo_dir: &str) -> Option<String>;

    /// List all git worktrees for the repository.
    async fn list_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String>;

    /// Detect the worktree strategy and return it.
    async fn detect_worktree_strategy(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<WorktreeStrategy, String>;

    /// Run clone operation.
    async fn clone_repository(
        &self,
        machine_id: Option<&str>,
        provider_id: &str,
        repo_path: &str,
        target_dir: &str,
    ) -> Result<(), String>;

    /// Create a feature branch.
    async fn create_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch_name: &str,
    ) -> Result<(), String>;

    /// Provision a subtask worktree.
    async fn provision_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<String, String>;

    /// Clean up a subtask worktree.
    async fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<(), String>;

    /// Delete a branch (and optionally any subtask branches and prune worktrees).
    async fn branch_delete(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
    ) -> Result<(), String>;

    /// Precheck if merging would succeed or fail.
    async fn precheck_merge(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        target_branch: &str,
        source_branch: &str,
    ) -> Result<MergePreCheck, String>;

    /// Merge a subtask branch.
    async fn merge_subtask(
        &self,
        machine_id: Option<&str>,
        worktree_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<(), String>;

    /// Sync feature branch with upstream default branch.
    async fn sync_feature_with_upstream(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        default_branch: &str,
    ) -> Result<SyncOutcome, SyncFailure>;
}
