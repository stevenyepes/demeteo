//! Worktree operations port.
//!
//! Provides abstract access to Git worktree operations such as cloning,
//! provisioning worktrees, checking repository state, and syncing with upstream.

use crate::domain::branch_listing::BranchOption;
use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// The caller-controlled half of a terminal-worktree creation.
///
/// Bundled rather than passed positionally: all three travel together from the
/// Tauri command through application policy into the adapter, and every one of
/// them is a `String` the user typed or picked. Two adjacent untrusted strings
/// of the same type is a swap the compiler cannot see.
#[derive(Debug, Clone)]
pub struct TerminalWorktreeRequest {
    /// The new branch to create. Must not already exist.
    pub branch: String,
    /// The branch to cut from. `None` leaves the start point at the primary
    /// checkout's HEAD, which is whatever the user last left it on.
    pub base_branch: Option<String>,
    /// The directory name below the repository's terminal area.
    pub worktree_name: String,
}

/// A terminal worktree that now exists, and where its branch came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalWorktreeCreated {
    pub worktree: WorktreeInfo,
    /// The start point Git was actually given: `origin/<base>` once the fetch
    /// reached origin, the local `<base>` when it did not, `HEAD` when the
    /// request named no base.
    ///
    /// Reported rather than inferred, because it is the whole answer to "am I
    /// starting from something stale" — and the fallback to a local ref happens
    /// exactly when the network was the thing that failed, which is when a
    /// caller assuming `origin/<base>` would be most wrong.
    pub base_ref: String,
}

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

/// Result of collapsing a feature branch's commits into one.
#[derive(Debug, Clone, PartialEq)]
pub enum SquashOutcome {
    /// The branch now points at a single new commit.
    Squashed {
        sha: String,
        /// How many commits were collapsed.
        collapsed: u32,
        /// Ref holding the pre-squash tip, so the rewrite is undoable.
        backup_ref: String,
    },
    /// The branch adds nothing to the default branch (no net change), so
    /// there is nothing to squash and nothing worth opening a PR for.
    NothingToSquash,
}

/// The repo's `commit-msg` hook rejected a proposed message.
#[derive(Debug, Clone)]
pub struct CommitMessageRejected {
    /// The hook's own output (e.g. commitlint's rule list) — fed back to
    /// the agent so it can repair the message.
    pub hook_output: String,
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

    /// Create a user-requested linked worktree without altering or reclaiming
    /// any existing worktree state.
    ///
    /// This creates a **new** branch. Supplying a branch that already exists is
    /// an error; implementations must not reuse, reset, or check out that
    /// branch in a new worktree.
    ///
    /// When the request names a base, implementations fetch it from origin
    /// first and cut from `origin/<base>`, falling back to the local `<base>`
    /// only when origin has no such ref. The primary checkout's HEAD is never
    /// silently used as the start point in that case: a session started from a
    /// stale base is the failure this is here to prevent, so the ref actually
    /// used comes back in [`TerminalWorktreeCreated::base_ref`].
    ///
    /// `project_root` is the Demeteo-owned root of the project and must
    /// already exist on the target host; implementations derive the worktree
    /// destination *below* it. Every field of `request` stays untrusted — the
    /// caller never gets to choose an absolute destination.
    async fn create_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        request: &TerminalWorktreeRequest,
    ) -> Result<TerminalWorktreeCreated, String>;

    /// Retire one terminal worktree the user is done with.
    ///
    /// `worktree_path` is untrusted — it arrives from a UI holding a listing
    /// that may be seconds out of date, and this method is one `git worktree
    /// remove --force` away from deleting a pipeline's checkout or the primary
    /// one. Implementations must therefore re-derive the terminal area from
    /// `project_root` and refuse any path
    /// [`list_terminal_worktrees`](Self::list_terminal_worktrees) would not
    /// have offered, rather than trusting the caller's path.
    ///
    /// The branch is deliberately left behind: the worktree is a directory the
    /// user can recreate, and the commits in it are not.
    ///
    /// `force` maps to git's own `--force`, which is what a worktree holding
    /// modified or untracked files needs. Without it, git's refusal comes back
    /// as the error — a caller is expected to surface that and let the user
    /// decide, never to retry with `force` on its own.
    async fn remove_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), String>;

    /// The branches a new terminal worktree may be based on.
    ///
    /// Read from refs already on the target host — no fetch. Opening a picker
    /// must not block on the network, and the fetch that makes a base current
    /// happens inside
    /// [`create_terminal_worktree`](Self::create_terminal_worktree), where the
    /// user has already committed to a base.
    async fn list_terminal_branches(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<BranchOption>, String>;

    /// The linked worktrees of `repo_dir` that belong to the terminal area
    /// below `project_root`, and nothing else.
    ///
    /// Separate from [`list_worktrees`](Self::list_worktrees) rather than a
    /// filter over it, because the classification needs the primary checkout's
    /// path and that method deliberately withholds it — three of its callers
    /// `worktree remove --force` whatever survives their own filter, so the
    /// main checkout must not be in what they filter.
    ///
    /// An error, not an empty list, when the area cannot be derived: an empty
    /// list is what a healthy repository with no terminal worktrees returns.
    async fn list_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
    ) -> Result<Vec<WorktreeInfo>, String>;

    /// Retire terminal worktrees left at the location this feature used before
    /// the area moved out of `repos/`, returning how many were unregistered.
    ///
    /// Unregistering is the point: those directories sit where
    /// `application::bootstrap` reclaims the tree, so a plain delete leaves
    /// `.git/worktrees` entries behind and a later add of the same name fails
    /// against a worktree Git still believes in. Implementations must leave the
    /// current-location worktrees alone.
    async fn cleanup_legacy_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<usize, String>;

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

    /// Run the repo's own `commit-msg` hook against a proposed message,
    /// without committing anything.
    ///
    /// This is how the finalize step lets a target repo's commitlint judge
    /// the squashed commit message *before* the message is used, so a
    /// rejection becomes feedback for the authoring agent instead of a
    /// failed commit. `Ok(())` when the repo installs no `commit-msg` hook.
    async fn validate_commit_message(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        message: &str,
    ) -> Result<(), CommitMessageRejected>;

    /// Collapse every commit the feature branch adds on top of the default
    /// branch into a single commit carrying `message`.
    async fn squash_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        default_branch: &str,
        message: &str,
    ) -> Result<SquashOutcome, String>;

    /// Restore a branch to its pre-squash tip from the backup ref written
    /// by [`squash_feature_branch`](Self::squash_feature_branch).
    async fn restore_pre_squash(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
    ) -> Result<(), String>;
}
