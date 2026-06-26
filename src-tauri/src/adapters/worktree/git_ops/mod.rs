use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{MergePreCheck, SyncFailure, SyncOutcome, WorktreeOpsPort};
use async_trait::async_trait;
use std::sync::Arc;

pub struct GitOpsHelper {
    pub(crate) app_settings: Arc<dyn AppSettingsRepository>,
    pub(crate) exec: Arc<dyn ExecutionPort>,
}

impl GitOpsHelper {
    pub fn new(app_settings: Arc<dyn AppSettingsRepository>, exec: Arc<dyn ExecutionPort>) -> Self {
        Self { app_settings, exec }
    }
}

pub(crate) mod clone;
pub(crate) mod health;
pub(crate) mod merge;
pub(crate) mod scope;
pub(crate) mod strategy;
pub(crate) mod sync;
pub(crate) mod worktree;

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops.rs"]
mod tests;

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

    async fn precheck_merge(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        target_branch: &str,
        source_branch: &str,
    ) -> Result<MergePreCheck, String> {
        self.precheck_merge(machine_id, repo_dir, target_branch, source_branch)
            .await
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
}
