//! SQLite-backed [`MergeExecutor`] implementation.
//!
//! Wraps `GitOpsHelper::sync_feature_with_upstream` with conflict
//! detection and `feature_syncs` audit rows. On a clean sync, the audit
//! row carries the merge commit SHA; on a conflict, the parsed file list
//! and raw stderr are stored as a JSON `ConflictReport` so the resolution
//! agent and the UI can render it.
//!
//! A blocked sync ([`crate::domain::sync_failure`]) is audited as
//! `status = 'blocked'` rather than as a conflict, and stores the serialized
//! [`UpstreamSyncFailure`] in the same column the conflict report uses: the
//! manual "Sync with main" path shows the reason once, in a banner the user then
//! dismisses, so a row saying only "an attempt failed at T" leaves nothing
//! behind to answer "why do this feature's syncs keep failing".
//!
//! The audit is not state, though, which is why every outcome also writes the
//! feature's single [`SyncSessionPort`] row. This is the only place that has
//! the repo dir, the machine, both branches and the verdict at once, so it is
//! where the live row is opened and closed; everything downstream — the
//! banner, the abort command, a later resolver — reads that row rather than
//! re-deriving any of it.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{
    ConflictReport, RepoContext, UpstreamSyncFailure, UpstreamSyncOutcome,
};
use crate::domain::sync_failure::SyncBlockedStage;
use crate::domain::sync_session::SyncSessionStatus;
use crate::paths;
use crate::ports::db::MergeAuditRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::sync_session::{SyncSession, SyncSessionPatch, SyncSessionPort};
use crate::ports::worktree_ops::SyncFailure;

pub struct SqliteMergeExecutor {
    merge_audit: Arc<dyn MergeAuditRepository>,
    sync_sessions: Arc<dyn SyncSessionPort>,
    git_ops: GitOpsHelper,
    exec: Arc<dyn ExecutionPort>,
    workspace_dir: std::path::PathBuf,
}

impl SqliteMergeExecutor {
    pub fn new(
        merge_audit: Arc<dyn MergeAuditRepository>,
        sync_sessions: Arc<dyn SyncSessionPort>,
        git_ops: GitOpsHelper,
        exec: Arc<dyn ExecutionPort>,
        workspace_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            merge_audit,
            sync_sessions,
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

        let machine_str = machine_id_opt
            .as_deref()
            .unwrap_or(crate::domain::ids::LOCAL_MACHINE)
            .to_string();
        let now = paths::now_ms();
        // Opened before the merge, not after it: a sync that is cut short
        // between here and its verdict is the case the row exists for, and one
        // written only on the way out would leave exactly that case invisible.
        let _ = self.sync_sessions.open(&SyncSession {
            feature_id: feature_id.0.clone(),
            machine_id: machine_str.clone(),
            repo_dir: repo_dir.clone(),
            feature_branch: feature_branch.to_string(),
            base_branch: default_branch.to_string(),
            status: SyncSessionStatus::Syncing,
            worktree_path: None,
            head_before: None,
            merge_commit_sha: None,
            conflict_files: Vec::new(),
            raw_error: None,
            attempts: 0,
            created_at: now,
            updated_at: now,
        });

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
                let _ = self
                    .exec
                    .run_command(
                        &machine_str,
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
                let _ = self.sync_sessions.update(
                    feature_id,
                    &SyncSessionPatch {
                        status: Some(if outcome.changed {
                            SyncSessionStatus::Merged
                        } else {
                            SyncSessionStatus::UpToDate
                        }),
                        head_before: Some(outcome.head_before.clone()),
                        merge_commit_sha: Some(Some(outcome.merge_commit_sha.clone())),
                        ..Default::default()
                    },
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
                head_before,
            }) => {
                let now = paths::now_ms();
                let _ = self.sync_sessions.update(
                    feature_id,
                    &SyncSessionPatch {
                        status: Some(SyncSessionStatus::Conflicted),
                        worktree_path: Some(worktree_path.clone()),
                        head_before: Some(head_before),
                        conflict_files: Some(files.clone()),
                        raw_error: Some(Some(raw_error.clone())),
                        ..Default::default()
                    },
                    now,
                );
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
            Err(SyncFailure::Blocked {
                stage,
                raw_error,
                worktree_path,
                head_before,
            }) => {
                // A blocked attempt can still have provisioned a tree — `Push`
                // always has, and it holds a real unpublished merge. Naming it on
                // the row is what lets `sync_abort` reclaim it; otherwise the only
                // thing that ever removes it is the next sync's force-remove.
                let _ = self.sync_sessions.update(
                    feature_id,
                    &SyncSessionPatch {
                        status: Some(SyncSessionStatus::Blocked),
                        raw_error: Some(Some(raw_error.clone())),
                        worktree_path: Some(worktree_path),
                        head_before: Some(head_before),
                        ..Default::default()
                    },
                    paths::now_ms(),
                );
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

    async fn record_sync_resolution(
        &self,
        feature_id: &FeatureId,
        resolution: &crate::domain::sync_session::SyncResolution,
    ) -> Result<(), String> {
        use crate::domain::sync_session::SyncResolution;
        self.sync_sessions.update(
            feature_id,
            &SyncSessionPatch {
                status: Some(resolution.status()),
                merge_commit_sha: match resolution {
                    SyncResolution::Succeeded { merge_commit_sha } => {
                        Some(Some(merge_commit_sha.clone()))
                    }
                    _ => None,
                },
                // A resolved sync has had its worktree discarded, and a row
                // still naming it reads back as an abandoned sync: the probe
                // finds the directory gone, which is the one observation
                // `reconcile` treats as terminal. Clearing it is also what stops
                // `sync_abort` aiming a delete at a path something else may have
                // re-provisioned since.
                worktree_path: match resolution {
                    SyncResolution::Succeeded { .. } => Some(None),
                    _ => None,
                },
                raw_error: match resolution {
                    SyncResolution::Failed { reason } => Some(Some(reason.clone())),
                    _ => None,
                },
                ..Default::default()
            },
            paths::now_ms(),
        )
    }

    async fn sync_session(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<crate::ports::sync_session::SyncSession>, String> {
        crate::application::sync_session::get_reconciled(
            &self.sync_sessions,
            &self.exec,
            feature_id,
        )
        .await
    }
}
