use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{
    CommitMessageRejected, SquashOutcome, SyncFailure, SyncOutcome, WorktreeOpsPort,
};
use async_trait::async_trait;
use std::sync::Arc;

/// Git plumbing shared by local, desktop-over-SSH, and runner execution.
///
/// **Shell-context audit (C1.3, `docs/EXECUTION_PARITY.md`).** Every
/// `exec.run_command` here deliberately uses the *non-login* default
/// [`ShellOptions`](crate::ports::execution::ShellOptions): these commands
/// invoke the system `git` binary, which lives on the default `PATH` of both a
/// local `sh -c` and a bare SSH channel, so no login profile is required and
/// both transports resolve it identically (D2). Working directories are always
/// passed explicitly (absolute paths, `git -C`, or a `cd …` prefix) — never the
/// ambient process cwd. Toolchain-managed tools that *do* need the user's login
/// profile (`mise`/`asdf`/`nvm` shims) never run through here; they run through
/// the login-shell paths — the agent spawn (`spawn_interactive`), the harness
/// gate (`step_executor::harness_shell::harness_shell_options`), and remote agent install.
#[derive(Clone)]
pub struct GitOpsHelper {
    pub(crate) app_settings: Arc<dyn AppSettingsRepository>,
    pub(crate) exec: Arc<dyn ExecutionPort>,
}

impl GitOpsHelper {
    pub fn new(app_settings: Arc<dyn AppSettingsRepository>, exec: Arc<dyn ExecutionPort>) -> Self {
        Self { app_settings, exec }
    }
}

/// The branch a subtask worktree is checked out on.
///
/// Provisioning, merge-back, cleanup, and the `ConflictDetected` payload all
/// have to agree on this name — a mismatch in any one of them silently
/// targets a branch that does not exist.
pub fn subtask_branch_name(feature_branch: &str, subtask_id: &str) -> String {
    format!("{}_subtask_{}", feature_branch, subtask_id)
}

pub(crate) mod clone;
pub(crate) mod health;
pub(crate) mod merge;
pub(crate) mod scope;
pub(crate) mod squash;
pub(crate) mod strategy;
pub(crate) mod sync;
pub(crate) mod worktree;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/common.rs"]
mod common;

#[async_trait]
impl WorktreeOpsPort for GitOpsHelper {
    async fn check_repo_dirty(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String> {
        self.check_repo_dirty(machine_id, repo_dir).await
    }

    async fn get_head_branch(&self, machine_id: Option<&str>, repo_dir: &str) -> Option<String> {
        self.get_head_branch(machine_id, repo_dir).await
    }

    async fn list_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        self.list_worktrees(machine_id, repo_dir).await
    }

    async fn create_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        worktree_name: &str,
    ) -> Result<WorktreeInfo, String> {
        self.create_terminal_worktree(machine_id, repo_dir, branch, worktree_name)
            .await
    }

    async fn detect_worktree_strategy(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<WorktreeStrategy, String> {
        self.detect_worktree_strategy(machine_id, repo_dir).await
    }

    async fn clone_repository(
        &self,
        machine_id: Option<&str>,
        provider_id: &str,
        repo_path: &str,
        target_dir: &str,
    ) -> Result<(), String> {
        self.clone_repository(machine_id, provider_id, repo_path, target_dir)
            .await
    }

    async fn create_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        self.create_feature_branch(machine_id, repo_dir, default_branch, branch_name)
            .await
    }

    async fn provision_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<String, String> {
        self.provision_subtask_worktree(machine_id, repo_dir, branch, subtask_id)
            .await
    }

    async fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        self.cleanup_subtask_worktree(machine_id, repo_dir, branch, subtask_id)
            .await
    }

    async fn branch_delete(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
    ) -> Result<(), String> {
        self.branch_delete(machine_id, repo_dir, branch).await
    }

    async fn merge_subtask(
        &self,
        machine_id: Option<&str>,
        worktree_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        self.merge_subtask(machine_id, worktree_dir, branch, subtask_id)
            .await
    }

    async fn sync_feature_with_upstream(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        default_branch: &str,
    ) -> Result<SyncOutcome, SyncFailure> {
        self.sync_feature_with_upstream(machine_id, repo_dir, feature_branch, default_branch)
            .await
    }

    async fn validate_commit_message(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        message: &str,
    ) -> Result<(), CommitMessageRejected> {
        self.validate_commit_message(machine_id, repo_dir, message)
            .await
    }

    async fn squash_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        default_branch: &str,
        message: &str,
    ) -> Result<SquashOutcome, String> {
        self.squash_feature_branch(
            machine_id,
            repo_dir,
            feature_branch,
            default_branch,
            message,
        )
        .await
    }

    async fn restore_pre_squash(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
    ) -> Result<(), String> {
        self.restore_pre_squash(machine_id, repo_dir, feature_branch)
            .await
    }
}
