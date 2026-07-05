//! SQLite-backed [`MergeExecutor`] implementation.
//!
//! Wraps `GitOpsHelper::merge_subtask` with conflict detection and
//! `subtask_merges` audit rows. On a clean merge, the audit row is
//! updated with the merge commit SHA; on a conflict, the parsed
//! file list + raw stderr is stored as a JSON `ConflictReport` so
//! downstream resolvers and the UI can render the cascade.

use std::sync::Arc;

use async_trait::async_trait;

use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{
    ConflictFile, ConflictReport, MergeOutcome, RepoContext, UpstreamSyncFailure,
    UpstreamSyncOutcome, WorktreeContext,
};
use crate::paths;
use crate::ports::db::MergeAuditRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::worktree_ops::MergePreCheck;

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
    #[allow(clippy::result_large_err)]
    async fn merge_subtask_into_feature(
        &self,
        feature_id: &FeatureId,
        source_branch: &str,
        target_branch: &str,
        subtask_run_id: &str,
    ) -> Result<MergeOutcome, ConflictReport> {
        // 1. Resolve the worktree + repo context from the subtask_run_id.
        //    The worktree is private to this subtask — no race with other
        //    pipelines. The repo_dir is still needed for the precheck
        //    (ref/object operations only, no checkout).
        let WorktreeContext {
            compute_type,
            remote_host,
            project_id,
            repo_path,
            worktree_path: wt_path,
        } = match self
            .merge_audit
            .lookup_worktree_context(feature_id, subtask_run_id)
        {
            Ok(v) => v,
            Err(e) => {
                return Err(ConflictReport {
                    source_branch: source_branch.to_string(),
                    target_branch: target_branch.to_string(),
                    files: vec![],
                    raw_error: format!("Failed to resolve worktree context: {}", e),
                    detected_at: paths::now_ms(),
                    worktree_path: None,
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
                    return Err(ConflictReport {
                        source_branch: source_branch.to_string(),
                        target_branch: target_branch.to_string(),
                        files: vec![],
                        raw_error: format!("Failed to resolve repo directory: {}", e),
                        detected_at: paths::now_ms(),
                        worktree_path: None,
                    });
                }
            }
        };
        let subtask_id = extract_subtask_id(source_branch).unwrap_or_else(|| "sub".to_string());
        let machine_str = machine_id_opt.as_deref().unwrap_or("local");
        let subtask_branch = format!("{}_subtask_{}", target_branch, subtask_id);
        let now = paths::now_ms();

        // 2. Pre-check: already merged?  Uses `merge-base` on remote
        //    refs — no working tree touched.
        let precheck = self
            .git_ops
            .precheck_merge(Some(machine_str), &repo_dir, target_branch, &subtask_branch)
            .await;

        if let Ok(MergePreCheck::AlreadyMerged) = precheck {
            let sha = self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} rev-parse refs/remotes/origin/{}",
                        paths::shell_escape_posix(&repo_dir),
                        paths::shell_escape_posix(target_branch),
                    ),
                )
                .await
                .ok()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".to_string());

            let outcome = MergeOutcome {
                merge_commit_sha: sha,
                source_branch: source_branch.to_string(),
                target_branch: target_branch.to_string(),
                already_merged: true,
            };

            let _ = self.merge_audit.record_merge_outcome(
                subtask_run_id,
                feature_id,
                source_branch,
                target_branch,
                "ok",
                Some(&outcome.merge_commit_sha),
                None,
                now,
            );

            return Ok(outcome);
        }

        // 3. Merge in the worktree (private, no shared-checkout race).
        let merge_result = self
            .git_ops
            .merge_subtask(Some(machine_str), &wt_path, target_branch, &subtask_id)
            .await;

        match merge_result {
            Ok(()) => {
                let sha = self
                    .exec
                    .run_command(
                        machine_str,
                        &format!(
                            "git -C {} rev-parse HEAD",
                            paths::shell_escape_posix(&wt_path)
                        ),
                    )
                    .await
                    .ok()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "unknown".to_string());

                let outcome = MergeOutcome {
                    merge_commit_sha: sha,
                    source_branch: source_branch.to_string(),
                    target_branch: target_branch.to_string(),
                    already_merged: false,
                };

                let _ = self.merge_audit.record_merge_outcome(
                    subtask_run_id,
                    feature_id,
                    source_branch,
                    target_branch,
                    "ok",
                    Some(&outcome.merge_commit_sha),
                    None,
                    now,
                );

                Ok(outcome)
            }
            Err(raw_err) => {
                let files = list_unmerged_files(self.exec.as_ref(), machine_str, &wt_path).await;

                let report = ConflictReport {
                    source_branch: source_branch.to_string(),
                    target_branch: target_branch.to_string(),
                    files,
                    raw_error: raw_err,
                    detected_at: now,
                    worktree_path: Some(wt_path.clone()),
                };

                let json_blob = serde_json::to_string(&report).unwrap_or_else(|_| "{}".to_string());
                let _ = self.merge_audit.record_merge_outcome(
                    subtask_run_id,
                    feature_id,
                    source_branch,
                    target_branch,
                    "conflict",
                    None,
                    Some(&json_blob),
                    now,
                );

                Err(report)
            }
        }
    }

    async fn skip_merge(&self, subtask_run_id: &str, reason: &str) -> Result<(), String> {
        self.merge_audit.skip_merge(subtask_run_id, reason)
    }

    async fn abort_in_progress(&self, target_branch: &str) -> Result<(), String> {
        let _ = target_branch;
        Err(
            "abort_in_progress must be invoked through the executor that owns the ExecutionPort"
                .to_string(),
        )
    }

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
                return Err(UpstreamSyncFailure {
                    report: ConflictReport {
                        source_branch: format!("origin/{}", default_branch),
                        target_branch: feature_branch.to_string(),
                        files: Vec::new(),
                        raw_error: format!("Failed to resolve repo context: {}", e),
                        detected_at: paths::now_ms(),
                        worktree_path: None,
                    },
                    worktree_path: None,
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
                    return Err(UpstreamSyncFailure {
                        report: ConflictReport {
                            source_branch: format!("origin/{}", default_branch),
                            target_branch: feature_branch.to_string(),
                            files: Vec::new(),
                            raw_error: format!("Failed to resolve repo directory: {}", e),
                            detected_at: paths::now_ms(),
                            worktree_path: None,
                        },
                        worktree_path: None,
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
                let machine_str = machine_id_opt.as_deref().unwrap_or("local");
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
            Err(failure) => {
                let now = paths::now_ms();
                let report = ConflictReport {
                    source_branch: format!("origin/{}", default_branch),
                    target_branch: feature_branch.to_string(),
                    files: failure.files,
                    raw_error: failure.raw_error,
                    detected_at: now,
                    worktree_path: failure.worktree_path.clone(),
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
                Err(UpstreamSyncFailure {
                    report,
                    worktree_path: failure.worktree_path,
                })
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

/// Best-effort parse: `feature/<slug>_subtask_sub-1` → "sub-1".
fn extract_subtask_id(branch: &str) -> Option<String> {
    let idx = branch.rfind("_subtask_")?;
    Some(branch[idx + "_subtask_".len()..].to_string())
}

/// Run `git status --porcelain --untracked-files=no` and pull out the
/// `UU` / `AA` / `DD` / `UA` / `AU` / `DU` / `UD` lines (i.e. unmerged
/// paths). Each line is "<XY> <path>" — we map XY to a short human
/// kind label and return a `Vec<ConflictFile>`.
async fn list_unmerged_files(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<ConflictFile> {
    let raw = match exec
        .run_command(
            machine_id,
            &format!(
                "git -C {} status --porcelain --untracked-files=no",
                paths::shell_escape_posix(repo_dir)
            ),
        )
        .await
    {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    raw.lines()
        .filter_map(|line| {
            // porcelain v1 format: "XY path" where XY is two chars
            // (left = index, right = worktree). The path is quoted
            // if it contains special chars; we don't currently need
            // to unquote because the executor's git invocations
            // produce relative paths without spaces in practice.
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            // Only unmerged states.
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../../tests/infrastructure/merge.rs"]
mod tests;
