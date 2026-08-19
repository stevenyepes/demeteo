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
use crate::application::sync_turns::SyncTurns;
use crate::domain::ids::FeatureId;
use crate::domain::models::{
    ConflictReport, FeatureDrift, RepoContext, UpstreamSyncFailure, UpstreamSyncOutcome,
};
use crate::domain::sync_failure::SyncBlockedStage;
use crate::domain::sync_session::SyncSessionStatus;
use crate::paths;
use crate::ports::db::{FeatureRepository, MergeAuditRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::sync_session::{SyncSession, SyncSessionPatch, SyncSessionPort};
use crate::ports::worktree_ops::{SyncFailure, SyncWorktreeObserver};

pub struct SqliteMergeExecutor {
    merge_audit: Arc<dyn MergeAuditRepository>,
    sync_sessions: Arc<dyn SyncSessionPort>,
    features: Arc<dyn FeatureRepository>,
    turns: Arc<SyncTurns>,
    git_ops: GitOpsHelper,
    exec: Arc<dyn ExecutionPort>,
    workspace_dir: std::path::PathBuf,
}

/// Writes the merge worktree onto the session the moment git hands one over.
///
/// The row is opened before the fetch and everything after it can be cut short,
/// so this is the difference between an interrupted sync leaving a row that
/// names its tree — probeable, reconcilable, abortable — and one that names
/// nothing and can only be reclaimed by the next sync's force-remove.
///
/// Writing it eagerly is also what makes the *success* path owe a clear: the
/// clean merge removes that tree on its way out, and a row still naming it
/// sends the pane's "Sync worktree" section at a directory the sync deleted.
struct RecordWorktree<'a> {
    sessions: &'a Arc<dyn SyncSessionPort>,
    feature_id: &'a FeatureId,
    /// What `provisioned` was told, for the caller that has to tidy up after
    /// it. The path is knowable here and, for an outcome that carries none,
    /// nowhere else.
    path: std::sync::Mutex<Option<String>>,
}

impl RecordWorktree<'_> {
    fn new<'a>(
        sessions: &'a Arc<dyn SyncSessionPort>,
        feature_id: &'a FeatureId,
    ) -> RecordWorktree<'a> {
        RecordWorktree {
            sessions,
            feature_id,
            path: std::sync::Mutex::new(None),
        }
    }

    fn provisioned_path(&self) -> Option<String> {
        self.path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl SyncWorktreeObserver for RecordWorktree<'_> {
    fn provisioned(&self, worktree_path: &str) {
        *self
            .path
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(worktree_path.to_string());
        let _ = self.sessions.update(
            self.feature_id,
            &SyncSessionPatch {
                worktree_path: Some(Some(worktree_path.to_string())),
                ..Default::default()
            },
            paths::now_ms(),
        );
    }
}

impl SqliteMergeExecutor {
    pub fn new(
        merge_audit: Arc<dyn MergeAuditRepository>,
        sync_sessions: Arc<dyn SyncSessionPort>,
        features: Arc<dyn FeatureRepository>,
        turns: Arc<SyncTurns>,
        git_ops: GitOpsHelper,
        exec: Arc<dyn ExecutionPort>,
        workspace_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            merge_audit,
            sync_sessions,
            features,
            turns,
            git_ops,
            exec,
            workspace_dir,
        }
    }

    /// The ports every session read here goes through.
    fn sync_ports(&self) -> crate::application::sync_session::SyncPorts<'_> {
        crate::application::sync_session::SyncPorts {
            sessions: &self.sync_sessions,
            exec: &self.exec,
            features: &self.features,
            turns: &self.turns,
        }
    }

    /// Whether the row may stop naming the tree a clean sync provisioned —
    /// `Some(None)` to clear it, `None` to leave it exactly as it is.
    ///
    /// A clean merge force-removes its throwaway worktree on the way out, so a
    /// row that keeps naming it points the pane at a directory that is not
    /// there. The delete is best-effort and reports nothing, though, and this
    /// is the last reader that will ever look at this session — `merged` and
    /// `up_to_date` are terminal — so a column cleared on a delete nobody
    /// confirmed would leave a directory on disk that no row names, which is
    /// the leak V43 closed. Same rule as
    /// [`close_session`](crate::application::sync_session::abort): the verdict
    /// comes from a probe, never from the teardown.
    ///
    /// `worktree == repo_dir` is the case with nothing to confirm:
    /// `provision_sync_worktree` returns the clone when the feature branch is
    /// already checked out there, nothing is removed, and the clone is not a
    /// sync worktree to tell anybody about.
    async fn swept_worktree(
        &self,
        machine: &str,
        repo_dir: &str,
        observer: &RecordWorktree<'_>,
    ) -> Option<Option<String>> {
        match observer.provisioned_path() {
            None => None,
            Some(path) if path == repo_dir => Some(None),
            Some(path) => crate::application::sync_session::worktree_confirmed_gone(
                &*self.exec,
                machine,
                &path,
            )
            .await
            .then_some(None),
        }
    }

    /// The machine and repository directory this feature's git work happens in.
    ///
    /// `None` for the machine means the local subprocess transport; every other
    /// value is a host the same `ExecutionPort` reaches over SSH, so no caller
    /// of this ever branches on which it got.
    async fn repo_target(
        &self,
        feature_id: &FeatureId,
    ) -> Result<(Option<String>, String), String> {
        let RepoContext {
            compute_type,
            remote_host,
            project_id,
            repo_path,
        } = self
            .merge_audit
            .lookup_repo_context(feature_id)
            .map_err(|e| format!("Failed to resolve repo context: {}", e))?;

        let local = compute_type.eq_ignore_ascii_case("local");
        let machine_id_opt = if local { None } else { remote_host.clone() };
        let repo_dir = if local {
            paths::repo_target_dir_local(&self.workspace_dir, &project_id, &repo_path)
                .to_string_lossy()
                .to_string()
        } else {
            paths::repo_target_dir_str(
                &self.exec,
                &compute_type,
                remote_host.as_deref(),
                &project_id,
                &repo_path,
                None,
            )
            .await
            .map_err(|e| format!("Failed to resolve repo directory: {}", e))?
        };
        Ok((machine_id_opt, repo_dir))
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
        // Before anything is resolved or fetched, because the row this would
        // overwrite is the only copy of a resolution nobody has read yet
        // ([`resync_refusal`](crate::domain::sync_session::resync_refusal)). A
        // row that could not be read is not evidence there is none, so it
        // refuses on the same terms rather than syncing over a maybe.
        match crate::application::sync_session::get_reconciled(self.sync_ports(), feature_id).await
        {
            Ok(Some(existing)) => {
                if let Some(refusal) = crate::domain::sync_session::resync_refusal(
                    existing.status,
                    existing.pushed_at.is_some(),
                    existing.blocked_stage,
                ) {
                    return Err(UpstreamSyncFailure::Blocked {
                        stage: SyncBlockedStage::HeldResolution,
                        raw_error: refusal.to_string(),
                    });
                }
            }
            Ok(None) => {}
            Err(e) => {
                return Err(UpstreamSyncFailure::Blocked {
                    stage: SyncBlockedStage::HeldResolution,
                    raw_error: format!(
                        "This feature's sync session could not be read, so whether the last \
                         resolution is still unpublished is unknown and nothing was synced: {}",
                        e
                    ),
                });
            }
        }

        // After the read above and before anything that touches the working
        // tree. `provision_sync_worktree` sweeps every `_wt_sync` worktree
        // checked out on this branch with `worktree remove --force` and
        // `remove_dir_all`, and the tree an out-of-band resolution is being
        // written in is exactly one of those — so the slot is taken here,
        // where every entry point passes, rather than at one of them. The
        // workflow's own `sync` node reached this function without one.
        //
        // Nothing has been written to the session yet, and nothing may be: the
        // row belongs to whichever turn holds the slot.
        let Some(_turn) = self.turns.claim(&feature_id.0, None) else {
            return Err(UpstreamSyncFailure::Blocked {
                stage: SyncBlockedStage::TurnInFlight,
                raw_error: "A sync or resolution is already running for this feature; \
                            wait for it to finish, or stop it first."
                    .to_string(),
            });
        };

        let (machine_id_opt, repo_dir) = match self.repo_target(feature_id).await {
            Ok(v) => v,
            Err(e) => {
                return Err(UpstreamSyncFailure::Blocked {
                    stage: SyncBlockedStage::RepoContext,
                    raw_error: e,
                });
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
            blocked_stage: None,
            pushed_at: None,
            attempts: 0,
            created_at: now,
            updated_at: now,
        });

        // Delegate the git work to GitOpsHelper and translate the
        // SyncOutcome / SyncFailure into the upstream-sync domain
        // types. The repo context is already resolved; the helper
        // doesn't need to look it up again.
        let observer = RecordWorktree::new(&self.sync_sessions, feature_id);
        match self
            .git_ops
            .sync_feature_with_upstream(
                machine_id_opt.as_deref(),
                &repo_dir,
                feature_branch,
                default_branch,
                &observer,
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
                    outcome.merge_commit_sha.as_deref(),
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
                        merge_commit_sha: Some(outcome.merge_commit_sha.clone()),
                        worktree_path: self
                            .swept_worktree(&machine_str, &repo_dir, &observer)
                            .await,
                        blocked_stage: Some(None),
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
                        blocked_stage: Some(None),
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
                merge_commit_sha,
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
                        // Written in both directions rather than only when
                        // there is one: this row is an upsert over whatever the
                        // last attempt left, and a stage inherited from it
                        // would say a merge is waiting on the branch when the
                        // fetch never even ran.
                        blocked_stage: Some(Some(stage)),
                        merge_commit_sha: Some(merge_commit_sha),
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

    async fn feature_drift(
        &self,
        feature_id: &FeatureId,
        feature_branch: &str,
        base_branch: &str,
        refresh: bool,
    ) -> Result<FeatureDrift, String> {
        let (machine_id_opt, repo_dir) = self.repo_target(feature_id).await?;
        let machine_str = machine_id_opt
            .as_deref()
            .unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let base_ref = format!("origin/{}", base_branch);

        let fetched = refresh
            && crate::adapters::worktree::git_ops::divergence::refresh_base_ref(
                &*self.exec,
                machine_str,
                &repo_dir,
                base_branch,
            )
            .await;

        Ok(FeatureDrift {
            divergence: crate::adapters::worktree::git_ops::divergence::count_divergence(
                &*self.exec,
                machine_str,
                &repo_dir,
                &format!("refs/heads/{}", feature_branch),
                &base_ref,
            )
            .await,
            base_ref,
            fetched,
            checked_at: paths::now_ms(),
        })
    }

    async fn record_sync_resolution(
        &self,
        feature_id: &FeatureId,
        resolution: &crate::domain::sync_session::SyncResolution,
    ) -> Result<(), String> {
        let now = paths::now_ms();
        self.sync_sessions.update(
            feature_id,
            &SyncSessionPatch::from_resolution(resolution, now),
            now,
        )
    }

    async fn sync_session(
        &self,
        feature_id: &FeatureId,
    ) -> Result<Option<crate::ports::sync_session::SyncSession>, String> {
        crate::application::sync_session::get_reconciled(self.sync_ports(), feature_id).await
    }
}

#[cfg(test)]
#[path = "../../tests/infrastructure/merge.rs"]
mod tests;
