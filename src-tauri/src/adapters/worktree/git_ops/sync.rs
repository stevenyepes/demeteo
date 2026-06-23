use super::GitOpsHelper;
use crate::paths;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{SyncFailure, SyncOutcome};

impl GitOpsHelper {
    /// Fetch the latest state of `default_branch` from `origin` and
    /// hard-reset the local copy of that branch to match. This is the
    /// one-time "snapshot" call used at feature start to make sure the
    /// local default_branch doesn't fall behind after other PRs have
    /// been merged upstream.
    ///
    /// Idempotent and safe to re-invoke: it does not touch any feature
    /// branches, only the local `default_branch` ref.
    pub async fn ensure_default_branch_updated(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_branch = paths::shell_escape_posix(default_branch);
        let safe_dir = paths::shell_escape_posix(repo_dir);

        // 1. Fetch the latest refs from origin. The fetch is best-effort:
        //    if origin is unreachable, we leave the local branch alone and
        //    warn via stderr (which the executor surfaces to the UI logs).
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} fetch origin {}", safe_dir, safe_branch),
            )
            .await;

        // 2. Resolve the remote tracking branch (origin/<default>). If
        //    the ref doesn't exist (offline / no remote), bail with a
        //    soft error so the caller can decide to proceed with the
        //    local branch anyway.
        let tracking = format!("origin/{}", default_branch);
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --verify {}",
                    safe_dir,
                    paths::shell_escape_posix(&tracking)
                ),
            )
            .await
            .map_err(|_| {
                format!(
                    "Local default branch '{}' has no upstream on origin; \
                     proceeding with whatever is local.",
                    default_branch
                )
            })?;

        // 3. Switch the working tree to the default branch and reset it
        //    to match origin/<default>. We use `--hard` because we
        //    explicitly want the local branch to be a byte-for-byte
        //    copy of upstream before any feature branch is cut from it.
        //    Combined into one command to avoid an extra subprocess spawn.
        self.exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} checkout {} && git -C {} reset --hard {}",
                    safe_dir,
                    safe_branch,
                    safe_dir,
                    paths::shell_escape_posix(&tracking)
                ),
            )
            .await?;
        Ok(())
    }

    /// Merge `origin/<default_branch>` into `feature_branch`. This is
    /// the "rebase from the user's perspective" call: it does NOT
    /// rebase (which would rewrite history) — it creates a merge
    /// commit so any in-flight reviewers see a clear fork/join in the
    /// graph. If conflicts arise, returns the list of unmerged files
    /// and leaves the working tree in the conflicted state for the
    /// caller to resolve.
    ///
    /// The `Ok` variant returns the new HEAD commit SHA (so the
    /// caller can record the merge commit in the audit trail). The
    /// `Err` variant carries the unmerged file list and raw git error.
    pub async fn sync_feature_with_upstream(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        default_branch: &str,
    ) -> Result<SyncOutcome, SyncFailure> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let safe_default = paths::shell_escape_posix(default_branch);

        let tracking = format!("origin/{}", default_branch);
        let feat_ref = format!("refs/heads/{}", feature_branch);
        let safe_feat_ref = paths::shell_escape_posix(&feat_ref);

        // 1. Refresh remote refs. We use `git fetch <remote> <branch>`
        //    so the local `refs/remotes/origin/<default>` ref is
        //    updated to the latest upstream state. The fetch is
        //    *reported* on failure — silently swallowing it is what
        //    caused the "no conflicts detected" bug where a stale
        //    `origin/<default>` was used as the merge source.
        let fetch_outcome = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} fetch origin {}", safe_dir, safe_default),
            )
            .await;
        if let Err(fetch_err) = fetch_outcome {
            return Err(SyncFailure {
                files: Vec::new(),
                raw_error: format!(
                    "Could not fetch origin/{} from remote: {}. \
                     Check the project's remote URL and credentials.",
                    default_branch, fetch_err
                ),
                worktree_path: None,
            });
        }

        // 2. Verify `origin/<default>` exists locally. After a
        //    successful fetch this is guaranteed for any branch the
        //    remote actually has; if the project's default_branch
        //    setting doesn't match a real upstream branch we surface
        //    that as a config error rather than a silent no-op.
        if self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --verify {}",
                    safe_dir,
                    paths::shell_escape_posix(&tracking)
                ),
            )
            .await
            .is_err()
        {
            return Err(SyncFailure {
                files: Vec::new(),
                raw_error: format!(
                    "Fetched origin but {} does not exist on the remote. \
                     The project's default_branch setting ('{}') may be wrong.",
                    tracking, default_branch
                ),
                worktree_path: None,
            });
        }

        // 3. Refs-only ops (no checkout needed). Use `refs/heads/<feature>`
        //    directly instead of `HEAD` to avoid touching the shared checkout.
        let head_before = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse {}", safe_dir, safe_feat_ref),
            )
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let _behind_count = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-list --count {}..{}",
                    safe_dir,
                    paths::shell_escape_posix(&tracking),
                    safe_feat_ref,
                ),
            )
            .await
            .ok()
            .map(|s| s.trim().parse::<u64>().unwrap_or(0))
            .unwrap_or(0);
        let ahead_count = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-list --count {}..{}",
                    safe_dir,
                    safe_feat_ref,
                    paths::shell_escape_posix(&tracking),
                ),
            )
            .await
            .ok()
            .map(|s| s.trim().parse::<u64>().unwrap_or(0))
            .unwrap_or(0);

        // If origin/<default> is not ahead of the feature branch,
        // the feature is already up to date with upstream. No merge
        // is needed and the call is a true no-op.
        if ahead_count == 0 {
            return Ok(SyncOutcome {
                merge_commit_sha: head_before,
                changed: false,
            });
        }

        // Do the merge in a temporary worktree (not the main repo) so
        // concurrent features cannot race on the shared checkout.
        let wt_path = self
            .provision_sync_worktree(Some(machine_str), repo_dir, feature_branch)
            .await
            .map_err(|e| SyncFailure {
                files: Vec::new(),
                raw_error: e,
                worktree_path: None,
            })?;
        let safe_wt = paths::shell_escape_posix(&wt_path);
        let merge_out = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} merge {} -m \"Sync feature with origin/{}\"",
                    safe_wt,
                    paths::shell_escape_posix(&tracking),
                    default_branch
                ),
            )
            .await;

        let result = match merge_out {
            Ok(_) => {
                let head_after = self
                    .exec
                    .run_command(machine_str, &format!("git -C {} rev-parse HEAD", safe_wt))
                    .await
                    .ok()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                let changed = head_after != head_before;

                if changed {
                    // Push the successful clean merge to origin so remote MR is updated
                    let push_cmd = format!(
                        "git -C {} push origin {}",
                        safe_wt,
                        paths::shell_escape_posix(feature_branch)
                    );
                    if let Err(push_err) = self.exec.run_command(machine_str, &push_cmd).await {
                        return Err(SyncFailure {
                            files: Vec::new(),
                            raw_error: format!(
                                "Sync merge succeeded locally but pushing to origin failed: {}",
                                push_err
                            ),
                            worktree_path: None,
                        });
                    }
                }

                Ok(SyncOutcome {
                    merge_commit_sha: head_after.clone(),
                    changed,
                })
            }
            Err(raw) => {
                // The merge left the worktree in a conflicted state.
                // Parse `git status` in the worktree for the unmerged files.
                let files = parse_unmerged_files(&*self.exec, machine_str, &wt_path).await;
                Err(SyncFailure {
                    files,
                    raw_error: raw,
                    worktree_path: Some(wt_path.clone()),
                })
            }
        };

        // If we used the main repo directly (no worktree), skip cleanup.
        // Otherwise, on success, remove the temp worktree; on conflict,
        // leave it in place for the resolution agent.
        if wt_path != repo_dir && result.is_ok() {
            let _ = self
                .exec
                .run_command(
                    machine_str,
                    &format!("git -C {} worktree remove --force {}", safe_dir, safe_wt),
                )
                .await;
            let _ = self
                .exec
                .run_command(machine_str, &format!("rm -rf {}", safe_wt))
                .await;
            let _ = self
                .exec
                .run_command(machine_str, &format!("git -C {} worktree prune", safe_dir))
                .await;
        }

        result
    }

    /// Provision a temporary linked worktree for a sync merge.
    /// The worktree has `<feature_branch>` checked out.
    ///
    /// If `<feature_branch>` is already the currently checked-out
    /// branch in the main repo, returns `repo_dir` directly
    /// (no worktree needed). The caller MUST skip worktree
    /// cleanup when the returned path equals `repo_dir`.
    async fn provision_sync_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
    ) -> Result<String, String> {
        let machine_str = machine_id.unwrap_or("local");

        // If the main repo already has the feature branch checked
        // out, we can merge in place — no worktree needed.
        let current_branch = self
            .get_head_branch(Some(machine_str), repo_dir)
            .await
            .unwrap_or_default();
        if current_branch == feature_branch {
            return Ok(repo_dir.to_string());
        }

        let safe_dir = paths::shell_escape_posix(repo_dir);

        // Clean up any stale sync worktrees checked out on this branch
        if let Ok(worktrees) = self.list_worktrees(Some(machine_str), repo_dir).await {
            for wt in worktrees {
                if wt.branch.as_deref() == Some(feature_branch) && wt.path.contains("_wt_sync") {
                    let safe_wt_path = paths::shell_escape_posix(&wt.path);
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!(
                                "git -C {} worktree remove --force {}",
                                safe_dir, safe_wt_path
                            ),
                        )
                        .await;
                    let _ = self
                        .exec
                        .run_command(machine_str, &format!("rm -rf {}", safe_wt_path))
                        .await;
                }
            }
            let _ = self
                .exec
                .run_command(machine_str, &format!("git -C {} worktree prune", safe_dir))
                .await;
        }

        // Use a deterministic path for this feature branch's sync worktree
        let wt_path = format!("{}_wt_sync_{}", repo_dir, feature_branch.replace('/', "_"));
        let safe_wt = paths::shell_escape_posix(&wt_path);

        // Force remove any pre-existing worktree at that path to avoid collisions
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} worktree remove --force {}", safe_dir, safe_wt),
            )
            .await;
        let _ = self
            .exec
            .run_command(machine_str, &format!("rm -rf {}", safe_wt))
            .await;
        let _ = self
            .exec
            .run_command(machine_str, &format!("git -C {} worktree prune", safe_dir))
            .await;

        self.exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree add {} {}",
                    safe_dir,
                    safe_wt,
                    paths::shell_escape_posix(feature_branch)
                ),
            )
            .await
            .map_err(|e| {
                format!(
                    "Failed to create sync worktree for '{}': {}",
                    feature_branch, e
                )
            })?;

        Ok(wt_path)
    }
}

/// Walk `git status --porcelain` and pull out the unmerged paths.
/// Shared by the sync flow and the existing `merge_subtask` conflict
/// path so both produce the same `ConflictFile` shape.
pub(crate) async fn parse_unmerged_files(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<crate::domain::models::ConflictFile> {
    use crate::domain::models::ConflictFile;
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
        Err(_) => return Vec::new(),
    };
    raw.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
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
