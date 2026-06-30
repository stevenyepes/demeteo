use super::GitOpsHelper;
use crate::domain::models::WorktreeInfo;
use crate::paths;
use std::path::Path;

impl GitOpsHelper {
    /// Get the current HEAD branch for a repo directory
    pub async fn get_head_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Option<String> {
        let machine_str = machine_id.unwrap_or("local");
        self.exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --abbrev-ref HEAD",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Parse `git worktree list --porcelain` output for a repo directory.
    /// Returns a list of worktrees (excluding the main one) with their branch and lock status.
    pub async fn list_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        let machine_str = machine_id.unwrap_or("local");
        let output = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree list --porcelain",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await?;

        let mut worktrees = Vec::new();
        let mut current_path: Option<String> = None;
        let mut current_branch: Option<String> = None;
        let mut is_locked = false;
        // block_index tracks which worktree entry we are accumulating.
        // Index 0 is always the primary worktree (the main checkout); we skip it.
        let mut block_index: i32 = -1;

        let flush = |path: String,
                     branch: Option<String>,
                     locked: bool,
                     idx: i32,
                     out: &mut Vec<WorktreeInfo>| {
            if idx > 0 {
                out.push(WorktreeInfo {
                    path,
                    branch,
                    is_locked: locked,
                });
            }
        };

        for line in output.lines() {
            if line.starts_with("worktree ") {
                // Flush the previously accumulated block (if any)
                if let Some(path) = current_path.take() {
                    flush(
                        path,
                        current_branch.take(),
                        is_locked,
                        block_index,
                        &mut worktrees,
                    );
                }
                block_index += 1;
                current_path = Some(line.trim_start_matches("worktree ").to_string());
                current_branch = None;
                is_locked = false;
            } else if line.starts_with("branch ") {
                // Strip "branch refs/heads/" prefix; fall back to raw remainder
                current_branch = Some(
                    line.trim_start_matches("branch refs/heads/")
                        .trim_start_matches("branch ")
                        .to_string(),
                );
            } else if line.starts_with("locked") {
                is_locked = true;
            } else if line.is_empty() {
                // Blank line = end of a porcelain block; flush it
                if let Some(path) = current_path.take() {
                    flush(
                        path,
                        current_branch.take(),
                        is_locked,
                        block_index,
                        &mut worktrees,
                    );
                    is_locked = false;
                }
            }
        }
        // Flush the final block if it wasn't terminated by a blank line
        if let Some(path) = current_path.take() {
            flush(
                path,
                current_branch.take(),
                is_locked,
                block_index,
                &mut worktrees,
            );
        }

        Ok(worktrees)
    }

    /// Create a feature branch off the default branch in the main repo.
    pub async fn create_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let safe_default = paths::shell_escape_posix(default_branch);
        let safe_branch = paths::shell_escape_posix(branch_name);

        // Create/update the feature branch ref without checking it out.
        // `git branch -f <branch> <start>` is a ref-only operation — it never
        // moves HEAD, so the main repo stays on the default branch throughout
        // the entire pipeline run. All agent work happens in linked worktrees;
        // the main checkout must not be disturbed.
        //
        // Replaces `git checkout -b` which moved HEAD to the feature branch and
        // then raced with the background `ensure_default_branch_updated` call
        // that immediately ran `git checkout <default>`, leaving the repo on
        // the default branch after every feature start.
        let cmd = format!(
            "git -C {} branch -f {} {}",
            safe_dir, safe_branch, safe_default,
        );
        match self.exec.run_command(machine_str, &cmd).await {
            Ok(_) => Ok(()),
            Err(_) => {
                // Branch may already exist from a prior interrupted run.
                // Verify the ref is reachable; if so, we can proceed.
                let check = format!(
                    "git -C {} rev-parse --verify refs/heads/{}",
                    safe_dir, safe_branch,
                );
                self.exec
                    .run_command(machine_str, &check)
                    .await
                    .map(|_| ())
                    .map_err(|_| format!("Failed to create feature branch '{}'", branch_name))
            }
        }
    }

    /// Provision a linked worktree for a subtask branched off the main feature branch.
    /// Returns the absolute path to the provisioned worktree.
    ///
    /// Robust against the "already exists" failure mode: handles three
    /// leftover-state cases in order — registered worktree (interrupted run
    /// left it in `.git/worktrees/`), orphan directory (cleanup never
    /// happened but git metadata is clean), and stale branch metadata
    /// (`worktree prune` cleans up). Each cleanup step's error is logged
    /// but non-fatal so a partially-set-up state still makes forward
    /// progress; `git worktree add --force` is the final safety net.
    ///
    /// IMPORTANT: the artifact-scope fence (`apply_artifact_scope`) chmods
    /// protected paths in the worktree to `a-w`. `unlink()` (which `rm`
    /// uses) needs write permission on the **parent directory**, so an
    /// `a-w` `src/` blocks `rm -rf` from cleaning up the worktree. We
    /// restore `u+w` before the `rm -rf` step.
    pub async fn provision_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<String, String> {
        let machine_str = machine_id.unwrap_or("local");
        let wt_dir = format!("{}_wt_{}", repo_dir, subtask_id);
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);

        // 1. If a previous run registered this worktree with git,
        //    `git worktree remove --force` is the only reliable way
        //    to detach it. `rm -rf` alone leaves stale metadata
        //    behind, which makes the subsequent `add` fail with
        //    "'<path>' is already used by worktree at '<other>'".
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree remove --force {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(&wt_dir)
                ),
            )
            .await;

        // 2. Restore write permissions. The artifact-scope fence may
        //    have chmod'd protected paths to `a-w` in a previous
        //    run; `rm -rf` needs `+w` on each parent directory it
        //    traverses, so a leftover a-w `src/` blocks cleanup.
        //    Best-effort: if chmod itself fails (rare; e.g. the dir
        //    no longer exists), the subsequent rm still works.
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "chmod -R u+w {} 2>/dev/null || true",
                    paths::shell_escape_posix(&wt_dir)
                ),
            )
            .await;

        // 3. Belt-and-suspenders: if the dir exists but isn't a
        //    registered worktree (orphan from a crashed run), remove
        //    it. Propagate failures now — silently continuing made the
        //    previous bug where the next `git worktree add` failed
        //    with "'<path>' already exists" and the user had no idea
        //    why. If rm really can't remove the dir (locked file,
        //    permission, read-only mount), return a clear error so the
        //    caller can surface it.
        self.exec
            .run_command(
                machine_str,
                &format!("rm -rf {}", paths::shell_escape_posix(&wt_dir)),
            )
            .await
            .map_err(|e| {
                format!(
                    "provision_subtask_worktree: rm -rf {} failed: {}. \
                     The directory may be locked or owned by another user; \
                     manual cleanup required before this feature can retry.",
                    wt_dir, e
                )
            })?;

        // 4. Prune any stale worktree metadata left over from
        //    crashed runs.
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree prune",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await;

        // 5. Create the worktree. `--force` lets git overwrite any
        //    remaining state (e.g. a missing-but-registered dir) so
        //    this last step is the safety net.
        let cmd = format!(
            "git -C {} worktree add --force {} -b {} {}",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(&wt_dir),
            paths::shell_escape_posix(&subtask_branch),
            paths::shell_escape_posix(feature_branch)
        );
        match self.exec.run_command(machine_str, &cmd).await {
            Ok(_) => {}
            Err(_) => {
                // Fallback: branch may already exist from a prior
                // interrupted run. Checkout without -b.
                let fallback_cmd = format!(
                    "git -C {} worktree add --force {} {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(&wt_dir),
                    paths::shell_escape_posix(&subtask_branch)
                );
                self.exec.run_command(machine_str, &fallback_cmd).await?;
            }
        }
        Ok(wt_dir)
    }

    /// Clean up a linked worktree for a subtask, including its branch.
    pub async fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let wt_dir = format!("{}_wt_{}", repo_dir, subtask_id);
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);

        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree remove --force {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(&wt_dir)
                ),
            )
            .await;
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree prune",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await;
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!("rm -rf {}", paths::shell_escape_posix(&wt_dir)),
            )
            .await;
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} branch -D {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(&subtask_branch)
                ),
            )
            .await;
        Ok(())
    }

    /// Delete a branch, all of its subtask branches, remove matching worktrees,
    /// and prune stale worktree metadata.
    ///
    /// If `repo_dir` no longer exists on disk all git commands are skipped —
    /// the branch is effectively gone — and `Ok` is returned.
    pub async fn branch_delete(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let safe_branch = paths::shell_escape_posix(branch);

        // If the repo directory is gone, there's nothing to do — git would
        // fail with "fatal: cannot change to '<path>': No such file or directory".
        if !Path::new(repo_dir).exists() {
            return Ok(());
        }

        // 1. Delete the feature branch.
        let delete_cmd = format!("git -C {} branch -D {}", safe_dir, safe_branch);
        self.exec
            .run_command(machine_str, &delete_cmd)
            .await
            .map_err(|e| format!("Failed to delete branch '{}': {}", branch, e))?;

        // 2. Delete all subtask branches for this feature.
        let subtask_cmd = format!(
            "git -C {} branch --list '{}_subtask_*' | while IFS= read -r b; do git -C {} branch -D \"$b\" 2>/dev/null; done",
            safe_dir,
            safe_branch,
            safe_dir
        );
        let _ = self.exec.run_command(machine_str, &subtask_cmd).await;

        // 3. Remove worktree directories for subtasks of this feature.
        let prefix = format!("{}_subtask_", branch);
        if let Ok(worktrees) = self.list_worktrees(machine_id, repo_dir).await {
            for wt in &worktrees {
                let is_match = wt.branch.as_deref().is_some_and(|b| b.starts_with(&prefix));
                if is_match {
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!(
                                "git -C {} worktree remove --force {}",
                                safe_dir,
                                paths::shell_escape_posix(&wt.path)
                            ),
                        )
                        .await;
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!("rm -rf {}", paths::shell_escape_posix(&wt.path)),
                        )
                        .await;
                }
            }
        }

        // 4. Prune orphaned worktrees.
        let prune_cmd = format!("git -C {} worktree prune", safe_dir);
        let _ = self.exec.run_command(machine_str, &prune_cmd).await;

        Ok(())
    }

    /// Returns `true` when the branch HEAD has advanced past `base_ref` —
    /// i.e. the agent committed at least one new change since we captured
    /// the pre-step baseline. Returns `true` when `base_ref` is `None`
    /// (unknown baseline → don't block the validate step).
    ///
    /// Used for no-op detection: if false, the implement step ran but
    /// made no commits, so advancing to validate would just waste tokens.
    pub async fn has_new_commits(
        &self,
        machine_id: Option<&str>,
        target_dir: &str,
        base_ref: Option<&str>,
    ) -> bool {
        let Some(base) = base_ref else {
            // No baseline captured — we can't tell, so allow validate.
            return true;
        };
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(target_dir);
        // git rev-parse HEAD gives the current tip; compare it to the stored baseline SHA.
        let Ok(current_sha) = self
            .exec
            .run_command(machine_str, &format!("git -C {} rev-parse HEAD", safe_dir))
            .await
        else {
            // git failure → assume something happened, allow validate.
            return true;
        };
        current_sha.trim() != base.trim()
    }
}
