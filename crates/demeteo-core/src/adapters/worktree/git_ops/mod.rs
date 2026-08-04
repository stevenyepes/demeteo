use crate::domain::branch_listing::BranchOption;
use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ExecutionPort, ProgramRequest};
use crate::ports::worktree_ops::{
    CommitMessageRejected, SquashOutcome, SyncFailure, SyncOutcome, TerminalWorktreeCreated,
    TerminalWorktreeRequest, WorktreeOpsPort,
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

/// A `git -C <repo_dir> …` invocation.
///
/// **`core.autocrlf`, `core.eol` and `core.symlinks` must never appear as a
/// `-c key=value` override here or at a call site.** All three decide how the
/// index compares against the working tree, so an override present for one
/// command and absent for the next makes every tracked file read as modified
/// — opencode #27276, arrived at from the same reasoning that makes the
/// override look correct. Here that answer is read by
/// [`GitOpsHelper::verify_and_revert_out_of_scope_writes`], which would
/// classify the entire tree as out of scope and `git checkout` away the work
/// the step just did.
///
/// The line-ending answer is instead written **once**, persistently, into the
/// clone's own config (`git_ops::clone`), where the index and the working tree
/// are created agreeing with it and every linked worktree inherits it.
pub(super) fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    git_request_vec(repo_dir, args.into_iter().map(str::to_string).collect())
}

/// The variadic form of [`git_request`], whose forbidden overrides it shares.
pub(super) fn git_request_vec(repo_dir: &str, args: Vec<String>) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [vec!["-C".to_string(), repo_dir.to_string()], args].concat(),
        ..ProgramRequest::default()
    }
}

/// The branch a subtask worktree is checked out on.
///
/// Provisioning, merge-back, cleanup, and the `ConflictDetected` payload all
/// have to agree on this name — a mismatch in any one of them silently
/// targets a branch that does not exist.
pub fn subtask_branch_name(feature_branch: &str, subtask_id: &str) -> String {
    format!(
        "{feature_branch}{}{subtask_id}",
        crate::domain::ids::SUBTASK_BRANCH_INFIX
    )
}

pub(crate) mod clone;
pub(crate) mod health;
pub(crate) mod merge;
pub(crate) mod scope;
pub(crate) mod squash;
pub(crate) mod strategy;
pub(crate) mod sync;
pub(crate) mod trusted;
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

    // The four terminal operations answer an interactive picker, which shows
    // whatever string comes back and offers no second attempt at explaining it.
    // Their one shared precondition — the project's clone exists — is the one
    // Git states least usefully, so it is restated here rather than in each
    // operation, where the start-point probe would still get there first.
    async fn create_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        request: &TerminalWorktreeRequest,
    ) -> Result<TerminalWorktreeCreated, String> {
        match self
            .create_terminal_worktree(machine_id, repo_dir, project_root, request)
            .await
        {
            Ok(created) => Ok(created),
            Err(error) => Err(self
                .explain_missing_checkout(machine_id, repo_dir, error)
                .await),
        }
    }

    async fn remove_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), String> {
        match self
            .remove_terminal_worktree(machine_id, repo_dir, project_root, worktree_path, force)
            .await
        {
            Ok(()) => Ok(()),
            Err(error) => Err(self
                .explain_missing_checkout(machine_id, repo_dir, error)
                .await),
        }
    }

    async fn list_terminal_branches(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<BranchOption>, String> {
        match self.list_terminal_branches(machine_id, repo_dir).await {
            Ok(branches) => Ok(branches),
            Err(error) => Err(self
                .explain_missing_checkout(machine_id, repo_dir, error)
                .await),
        }
    }

    async fn list_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        match self
            .list_terminal_worktrees(machine_id, repo_dir, project_root)
            .await
        {
            Ok(worktrees) => Ok(worktrees),
            Err(error) => Err(self
                .explain_missing_checkout(machine_id, repo_dir, error)
                .await),
        }
    }

    async fn cleanup_legacy_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<usize, String> {
        self.cleanup_legacy_terminal_worktrees(machine_id, repo_dir)
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
