//! SQLite-backed [`MergeExecutor`] implementation.
//!
//! Wraps `GitOpsHelper::sync_feature_with_upstream` with conflict
//! detection and `feature_syncs` audit rows. On a clean sync, the audit
//! row carries the merge commit SHA; on a conflict, the parsed file list
//! and raw stderr are stored as a JSON `ConflictReport` so the resolution
//! agent and the UI can render it.
//!
//! A blocked sync ([`crate::domain::sync_failure`]) is audited as
//! `status = 'blocked'`, and that spelling is load-bearing:
//! `get_last_sync_worktree_path` answers from the newest `'conflict'` row, so
//! a blocked attempt filed as a conflict carries a `worktree_path` of `None`
//! that hides the real conflict's worktree from the resolver. Its own row
//! stores the serialized [`UpstreamSyncFailure`] in the same column the
//! conflict report uses: the manual "Sync with main" path shows the reason
//! once, in a banner the user then dismisses, so a row saying only "an attempt
//! failed at T" leaves nothing behind to answer "why do this feature's syncs
//! keep failing".

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{
    ConflictReport, RepoContext, UpstreamSyncFailure, UpstreamSyncOutcome,
};
use crate::domain::sync_failure::SyncBlockedStage;
use crate::paths;
use crate::ports::db::MergeAuditRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::worktree_ops::SyncFailure;

pub struct SqliteMergeExecutor {
    merge_audit: Arc<dyn MergeAuditRepository>,
    git_ops: GitOpsHelper,
    exec: Arc<dyn ExecutionPort>,
    workspace_dir: std::path::PathBuf,
}

impl SqliteMergeExecutor {
    pub fn new(
        merge_audit: Arc<dyn MergeAuditRepository>,
        git_ops: GitOpsHelper,
        exec: Arc<dyn ExecutionPort>,
        workspace_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            merge_audit,
            git_ops,
            exec,
            workspace_dir,
        }
    }
}

#[async_trait]
impl MergeExecutor for SqliteMergeExecutor {
    async fn sync_feature_with_upstream(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        default_branch: &str,
    ) -> Result<UpstreamSyncOutcome, UpstreamSyncFailure> {
        // Resolve the project / machine / repo dir from the feature row.
        let RepoContext {
            compute_type,
            remote_host,
            project_id,
            repo_path,
        } = match self.merge_audit.lookup_repo_context(feature_id) {
            Ok(v) => v,
            Err(e) => {
                return Err(UpstreamSyncFailure::Blocked {
                    stage: SyncBlockedStage::RepoContext,
                    raw_error: format!("Failed to resolve repo context: {}", e),
                });
            }
        };

        let machine_id_opt = if compute_type.eq_ignore_ascii_case("local") {
            None
        } else {
            remote_host.clone()
        };

        let repo_dir = if compute_type.eq_ignore_ascii_case("local") {
            paths::repo_target_dir_local(&self.workspace_dir, &project_id, &repo_path)
                .to_string_lossy()
                .to_string()
        } else {
            match paths::repo_target_dir_str(
                &self.exec,
                &compute_type,
                remote_host.as_deref(),
                &project_id,
                &repo_path,
                None,
            )
            .await
            {
                Ok(dir) => dir,
                Err(e) => {
                    return Err(UpstreamSyncFailure::Blocked {
                        stage: SyncBlockedStage::RepoContext,
                        raw_error: format!("Failed to resolve repo directory: {}", e),
                    });
                }
            }
        };

        // Delegate the git work to GitOpsHelper and translate the
        // SyncOutcome / SyncFailure into the upstream-sync domain
        // types. The repo context is already resolved; the helper
        // doesn't need to look it up again.
        match self
            .git_ops
            .sync_feature_with_upstream(
                machine_id_opt.as_deref(),
                &repo_dir,
                feature_branch,
                default_branch,
            )
            .await
        {
            Ok(outcome) => {
                let machine_str = machine_id_opt
                    .as_deref()
                    .unwrap_or(crate::domain::ids::LOCAL_MACHINE);
                let _ = self
                    .exec
                    .run_command(
                        machine_str,
                        &format!(
                            "git -C {} rev-parse HEAD",
                            paths::shell_escape_posix(&repo_dir)
                        ),
                    )
                    .await;
                let _ = self.merge_audit.record_sync_outcome(
                    feature_id,
                    feature_branch,
                    default_branch,
                    "ok",
                    Some(&outcome.merge_commit_sha),
                    None,
                    paths::now_ms(),
                );
                Ok(UpstreamSyncOutcome {
                    merge_commit_sha: outcome.merge_commit_sha,
                    changed: outcome.changed,
                    default_branch: default_branch.to_string(),
                })
            }
            Err(SyncFailure::Conflict {
                files,
                raw_error,
                worktree_path,
            }) => {
                let now = paths::now_ms();
                let report = ConflictReport {
                    source_branch: format!("origin/{}", default_branch),
                    target_branch: feature_branch.to_string(),
                    files,
                    raw_error,
                    detected_at: now,
                    worktree_path: worktree_path.clone(),
                };
                let json_blob = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
                let _ = self.merge_audit.record_sync_outcome(
                    feature_id,
                    feature_branch,
                    default_branch,
                    "conflict",
                    None,
                    Some(&json_blob),
                    now,
                );
                Err(UpstreamSyncFailure::Conflict {
                    report,
                    worktree_path,
                })
            }
            Err(SyncFailure::Blocked { stage, raw_error }) => {
                let failure = UpstreamSyncFailure::Blocked { stage, raw_error };
                let json_blob =
                    serde_json::to_string(&failure).unwrap_or_else(|_| "{}".to_string());
                let _ = self.merge_audit.record_sync_outcome(
                    feature_id,
                    feature_branch,
                    default_branch,
                    "blocked",
                    None,
                    Some(&json_blob),
                    paths::now_ms(),
                );
                Err(failure)
            }
        }
    }

    async fn get_last_sync_worktree_path(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<String>, String> {
        self.merge_audit.get_last_sync_worktree_path(feature_id)
    }
}
