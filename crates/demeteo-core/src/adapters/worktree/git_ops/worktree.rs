use super::GitOpsHelper;
use crate::domain::models::WorktreeInfo;
use crate::paths;
use std::path::{Component, Path, PathBuf};

impl GitOpsHelper {
    /// Get the current HEAD branch for a repo directory
    pub async fn get_head_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Option<String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
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

    /// Create a linked worktree for an interactive terminal session.
    ///
    /// Unlike subtask provisioning, this path never removes an existing
    /// worktree, resets a branch, or falls back to an existing branch. `git
    /// worktree add -b` creates the requested branch from the current
    /// primary-checkout HEAD; an existing requested branch is rejected rather
    /// than reused. The caller owns both names, so a collision is an error the
    /// user must resolve rather than stale pipeline state to reclaim.
    pub async fn create_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        worktree_name: &str,
    ) -> Result<WorktreeInfo, String> {
        validate_terminal_branch(branch)?;
        let destination = terminal_worktree_dir(repo_dir, worktree_name)?;
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let command = terminal_worktree_create_cmd(repo_dir, branch, &destination)?;
        self.exec
            .run_command(machine_str, &command)
            .await
            .map_err(|e| {
                format!(
                    "create_terminal_worktree: git worktree add for branch '{branch}' failed: {e}"
                )
            })?;

        Ok(WorktreeInfo {
            path: destination,
            branch: Some(branch.to_string()),
            is_locked: false,
        })
    }

    /// Create a feature branch off the default branch in the main repo.
    ///
    /// The start point is the **remote-tracking** ref `origin/<default>`
    /// when it exists, falling back to the local `<default>` branch. Cutting
    /// from `origin/<default>` (which the caller refreshes with
    /// `ensure_default_branch_updated` immediately before this) guarantees
    /// the feature branch is based on the latest upstream. `ensure_default_branch_updated`
    /// also tries to keep the local `<default>` ref itself in sync — via
    /// `update-ref` when HEAD is on another branch, or via `merge --ff-only`
    /// when HEAD is on `<default>` with a clean working tree — so the
    /// validate step doesn't see "extra changes not in main" later. The
    /// local fallback here keeps the offline / no-remote path working
    /// (where `origin/<default>` simply doesn't resolve).
    pub async fn create_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let safe_default = paths::shell_escape_posix(default_branch);
        let safe_tracking = paths::shell_escape_posix(&format!("origin/{}", default_branch));
        let safe_branch = paths::shell_escape_posix(branch_name);

        // Create/update the feature branch ref without checking it out.
        // `git branch -f <branch> <start>` is a ref-only operation — it never
        // moves HEAD, so the main repo stays on the default branch throughout
        // the entire pipeline run. All agent work happens in linked worktrees;
        // the main checkout must not be disturbed.
        //
        // Prefer `origin/<default>` as the start point (latest upstream);
        // fall back to the local `<default>` when there's no remote-tracking
        // ref (offline, no remote, brand-new repo).
        let from_origin = format!(
            "git -C {} branch -f {} {}",
            safe_dir, safe_branch, safe_tracking,
        );
        if self
            .exec
            .run_command(machine_str, &from_origin)
            .await
            .is_ok()
        {
            return Ok(());
        }

        let cmd = format!(
            "git -C {} branch -f {} {}",
            safe_dir, safe_branch, safe_default,
        );
        match self.exec.run_command(machine_str, &cmd).await {
            Ok(_) => Ok(()),
            Err(create_err) => {
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
                    // Surface the real git error from the *create* attempt (not
                    // the verify probe, whose "unknown revision" just means the
                    // branch legitimately doesn't exist yet). The create stderr
                    // carries the actionable cause — e.g. a start-point that
                    // doesn't exist locally (`not a valid object name: 'main'`
                    // when the repo's default is really `master`). Swallowing it
                    // left every failure looking identical.
                    .map_err(|_| {
                        format!(
                            "Failed to create feature branch '{}' off '{}': {}",
                            branch_name, default_branch, create_err
                        )
                    })
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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt_dir = worktree_dir(repo_dir, subtask_id);
        let subtask_branch = super::subtask_branch_name(feature_branch, subtask_id);

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
                // Fallback: the branch already exists from a prior interrupted
                // run, so `-b` refused. Check it out without `-b`.
                let fallback_cmd = format!(
                    "git -C {} worktree add --force {} {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(&wt_dir),
                    paths::shell_escape_posix(&subtask_branch)
                );
                self.exec.run_command(machine_str, &fallback_cmd).await?;

                // That branch still points at the *previous* attempt's tip —
                // it carries commits the failed attempt made, and the caller
                // has since reset the feature branch away from them. Left as
                // is, the new attempt would build on abandoned work and merge
                // all of it back, silently defeating the rollback. Provisioning
                // must hand back a worktree at the feature branch, exactly as
                // the `-b` path does.
                self.exec
                    .run_command(
                        machine_str,
                        &format!(
                            "git -C {} reset --hard {}",
                            paths::shell_escape_posix(&wt_dir),
                            paths::shell_escape_posix(feature_branch)
                        ),
                    )
                    .await?;
            }
        }

        // 6. `git worktree add` gives the subtask a clean checkout — it does
        //    not carry over gitignored dependency caches (`node_modules/`,
        //    `target/`, `.venv/`, …). Build/test harnesses run in this
        //    worktree during agent and verify steps and fail with missing
        //    dependencies otherwise. Symlink the well-known cache dirs from
        //    the primary checkout when present there, so the harness sees
        //    the same install without re-running `npm ci` / `cargo fetch`
        //    per subtask. Best-effort: a failed link here shouldn't block
        //    worktree provisioning — the step will simply see the missing
        //    dependency and the harness fails as before.
        let _ = self
            .exec
            .run_command(
                machine_str,
                &link_dependency_caches_cmd(
                    repo_dir,
                    &wt_dir,
                    &paths::feature_cache_dir(repo_dir, feature_branch),
                ),
            )
            .await;

        Ok(wt_dir)
    }

    /// Resolve `dir`'s current `HEAD` to a full sha. `None` when git cannot
    /// answer (not a repo, no commits yet, transport gone).
    ///
    /// Exists because a baseline is only evidence about the commit it names
    /// (HB2a): the producer must record the sha it **actually measured**, not
    /// the one it assumed it was on. `git rev-parse HEAD` inside the worktree
    /// is the only thing that can tell it apart.
    pub async fn head_sha(&self, machine_id: Option<&str>, dir: &str) -> Option<String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        self.exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse HEAD", paths::shell_escape_posix(dir)),
            )
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Provision a linked worktree **detached at `sha`** — no branch, no
    /// branch creation, nothing that could later be merged anywhere.
    ///
    /// [`provision_subtask_worktree`](Self::provision_subtask_worktree) cannot
    /// do this: it takes a *branch*, and creates a subtask branch off it. The
    /// baseline fallback (HB2b) needs the opposite — a checkout of a commit
    /// that predates the feature entirely, so the harness it runs there is
    /// measuring the base rather than the work under test. `git worktree add
    /// --detach <path> <sha>` is that primitive.
    ///
    /// Detached is not an implementation detail, it is the safety property: a
    /// worktree with no branch cannot be committed onto and cannot be merged
    /// back by anything, so a measurement can never contaminate the feature.
    ///
    /// `cache_dir`, when given, seeds the well-known dependency caches exactly
    /// as the subtask path does — a fresh checkout has no `node_modules` and no
    /// `target/`, and the caller is about to run `prepare_command` in it.
    ///
    /// Leftover-state handling mirrors `provision_subtask_worktree` step for
    /// step (registered worktree → write-restore → orphan dir → prune → add),
    /// because the failure modes are the same ones and an interrupted run
    /// leaves the same debris.
    pub async fn provision_detached_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        sha: &str,
        worktree_id: &str,
        cache_dir: Option<&str>,
    ) -> Result<String, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt_dir = worktree_dir(repo_dir, worktree_id);

        self.clear_worktree_path(machine_str, repo_dir, &wt_dir)
            .await
            .map_err(|e| format!("provision_detached_worktree: {e}"))?;

        let cmd = format!(
            "git -C {} worktree add --detach --force {} {}",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(&wt_dir),
            paths::shell_escape_posix(sha),
        );
        self.exec
            .run_command(machine_str, &cmd)
            .await
            .map_err(|e| {
                format!(
                    "provision_detached_worktree: git worktree add --detach at {sha} failed: {e}"
                )
            })?;

        if let Some(cache) = cache_dir {
            let _ = self
                .exec
                .run_command(
                    machine_str,
                    &link_dependency_caches_cmd(repo_dir, &wt_dir, cache),
                )
                .await;
        }

        Ok(wt_dir)
    }

    /// Tear down a worktree provisioned by
    /// [`provision_detached_worktree`](Self::provision_detached_worktree).
    ///
    /// Deliberately **not** `cleanup_subtask_worktree`: that one ends by
    /// deleting `<feature>_subtask_<id>`, and a detached worktree has no branch
    /// to delete. Asking git to `branch -D` a ref that never existed would be a
    /// guaranteed error on a path whose whole job is to leave nothing behind.
    ///
    /// Best-effort on every command, as the subtask cleanup is — the caller
    /// runs this on the success *and* failure paths and must not have its own
    /// outcome changed by a teardown hiccup.
    pub async fn cleanup_detached_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        worktree_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt_dir = worktree_dir(repo_dir, worktree_id);
        let _ = self
            .clear_worktree_path(machine_str, repo_dir, &wt_dir)
            .await;
        Ok(())
    }

    /// Get a worktree path back to "nothing is here": deregister it with git,
    /// restore write permissions the artifact-scope fence may have stripped,
    /// remove the directory, and prune stale metadata.
    ///
    /// Only the `rm -rf` propagates its error — the rest are best-effort for
    /// the reasons `provision_subtask_worktree` documents at length (each
    /// handles a *leftover* state that usually isn't there). A failed `rm`
    /// is different: it means the path is still occupied, so whatever the
    /// caller was about to create there cannot be created.
    async fn clear_worktree_path(
        &self,
        machine_str: &str,
        repo_dir: &str,
        wt_dir: &str,
    ) -> Result<(), String> {
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} worktree remove --force {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(wt_dir)
                ),
            )
            .await;
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "chmod -R u+w {} 2>/dev/null || true",
                    paths::shell_escape_posix(wt_dir)
                ),
            )
            .await;
        self.exec
            .run_command(
                machine_str,
                &format!("rm -rf {}", paths::shell_escape_posix(wt_dir)),
            )
            .await
            .map_err(|e| {
                format!(
                    "rm -rf {wt_dir} failed: {e}. The directory may be locked or owned by \
                     another user; manual cleanup is required."
                )
            })?;
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
        Ok(())
    }

    /// Clean up a linked worktree for a subtask, including its branch.
    ///
    /// IMPORTANT: the artifact-scope fence (`apply_artifact_scope`) chmods
    /// protected paths in the worktree to `a-w` for Verify/Artifacts/ReadOnly
    /// steps. `unlink()` (which both `git worktree remove` and `rm -rf` rely
    /// on) needs write permission on the **parent directory**, so an `a-w`
    /// `src/` left over from the step's own run silently blocks this cleanup
    /// — `git worktree remove --force` and `rm -rf` each fail partway
    /// through, and since every command here is best-effort (`let _ = ...`),
    /// the failure is swallowed and a gutted, git-disconnected directory
    /// skeleton is left on disk. Restore `u+w` first, exactly as
    /// [`Self::provision_subtask_worktree`]'s leftover-state cleanup already
    /// does.
    pub async fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt_dir = worktree_dir(repo_dir, subtask_id);
        let subtask_branch = super::subtask_branch_name(feature_branch, subtask_id);

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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let safe_dir = paths::shell_escape_posix(repo_dir);
        let safe_branch = paths::shell_escape_posix(branch);

        // If the repo directory is gone, there's nothing to do — git would
        // fail with "fatal: cannot change to '<path>': No such file or directory".
        if !Path::new(repo_dir).exists() {
            return Ok(());
        }

        // Order matters: a branch checked out in a worktree cannot be
        // deleted (`git branch -D` refuses with "checked out at …"), so we
        // must remove the worktrees *before* deleting their branches. This
        // mirrors `cleanup_subtask_worktree`'s worktree→branch ordering.

        // 1. Remove worktree directories for subtasks of this feature.
        let prefix = format!("{}_subtask_", branch);
        if let Ok(worktrees) = self.list_worktrees(machine_id, repo_dir).await {
            for wt in &worktrees {
                let is_match = wt.branch.as_deref().is_some_and(|b| b.starts_with(&prefix));
                if is_match {
                    // Restore write perms the artifact-scope fence may have
                    // stripped during the step's run — see the doc comment
                    // on `cleanup_subtask_worktree` for why this must come
                    // before both `worktree remove` and `rm -rf`.
                    let _ = self
                        .exec
                        .run_command(
                            machine_str,
                            &format!(
                                "chmod -R u+w {} 2>/dev/null || true",
                                paths::shell_escape_posix(&wt.path)
                            ),
                        )
                        .await;
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

        // 2. Prune orphaned worktree metadata so the refs are no longer
        //    considered "checked out" by the branch deletes below.
        let prune_cmd = format!("git -C {} worktree prune", safe_dir);
        let _ = self.exec.run_command(machine_str, &prune_cmd).await;

        // 3. Delete all subtask branches for this feature (now that their
        //    worktrees are gone). Use `--format=%(refname:short)` so the
        //    listing emits clean branch names — `git branch --list` prefixes
        //    each line with two spaces (or `* `), and `IFS= read` preserves
        //    that leading whitespace, so `git branch -D "  <name>"` would
        //    never match a real ref.
        let subtask_cmd = format!(
            "git -C {} branch --list '{}_subtask_*' --format='%(refname:short)' | while IFS= read -r b; do git -C {} branch -D \"$b\" 2>/dev/null; done",
            safe_dir,
            safe_branch,
            safe_dir
        );
        let _ = self.exec.run_command(machine_str, &subtask_cmd).await;

        // 4. Drop this feature's dependency-cache root. It is per-feature (see
        //    `paths::feature_cache_dir`), so nothing else can be using it once
        //    the feature is gone — and it holds a whole `node_modules` /
        //    `target`, which would otherwise leak once per feature, forever.
        let _ = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "rm -rf {}",
                    paths::shell_escape_posix(&paths::feature_cache_dir(repo_dir, branch))
                ),
            )
            .await;

        // 5. Delete the feature branch itself.
        let delete_cmd = format!("git -C {} branch -D {}", safe_dir, safe_branch);
        self.exec
            .run_command(machine_str, &delete_cmd)
            .await
            .map_err(|e| format!("Failed to delete branch '{}': {}", branch, e))?;

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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
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

    /// Resolve the commit where `branch` most recently diverged from
    /// `default_branch` — the feature's fork point. Used to compute a
    /// review diff that always covers the complete feature change,
    /// independent of how many `on_failure` retries have merged work
    /// back into `branch` since (a per-attempt base SHA, recaptured as
    /// `branch`'s current tip on each retry, already includes prior
    /// attempts' merged commits and so understates the diff).
    ///
    /// Returns `None` if either ref doesn't resolve or `git merge-base`
    /// fails (e.g. the two branches share no history) — callers fall
    /// back to their pre-existing per-attempt base.
    pub async fn merge_base(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch: &str,
    ) -> Option<String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let cmd = format!(
            "git -C {} merge-base {} {}",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(default_branch),
            paths::shell_escape_posix(branch),
        );
        self.exec
            .run_command(machine_str, &cmd)
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }
}

/// Where a linked worktree for `worktree_id` lives: a **sibling of the repo
/// directory**, named by suffixing it.
///
/// Deliberately not a `PathBuf::join` — nothing here descends into a
/// directory, so there is no separator to get wrong on any platform; the
/// suffix keeps every worktree out of the repo it was cut from, which is what
/// stops `git status` and every glob in the project from seeing them. Every
/// provisioner and every cleanup must agree on this string or a teardown
/// silently misses its target, so it is written once.
fn worktree_dir(repo_dir: &str, worktree_id: &str) -> String {
    format!("{}_wt_{}", repo_dir, worktree_id)
}

/// Resolve a terminal worktree beneath a directory controlled by Demeteo, not
/// by the interactive caller. A relative name may contain normal path
/// components, but it may never select the repository root, an absolute path,
/// or an ancestor of that directory.
fn terminal_worktree_dir(repo_dir: &str, worktree_name: &str) -> Result<String, String> {
    let name = Path::new(worktree_name);
    if worktree_name.trim().is_empty()
        || name.is_absolute()
        // A remote repository may use POSIX paths while the desktop host is
        // Windows (or vice versa), so reject the other platform's absolute
        // path forms instead of trusting this host's `Path` parser alone.
        || worktree_name.contains('\\')
        || worktree_name.contains(':')
        || name.components().any(|component| {
            matches!(
                component,
                Component::ParentDir
                    | Component::RootDir
                    | Component::Prefix(_)
                    | Component::CurDir
            )
        })
    {
        return Err("create_terminal_worktree: worktree name must be a non-empty relative path without traversal".to_string());
    }

    let worktree_area = terminal_worktree_area(repo_dir)?;
    let destination: PathBuf = worktree_area.join(name);
    Ok(destination.to_string_lossy().into_owned())
}

fn terminal_worktree_area(repo_dir: &str) -> Result<PathBuf, String> {
    let repo = Path::new(repo_dir);
    let parent = repo.parent().ok_or_else(|| {
        "create_terminal_worktree: repository path has no trusted parent directory".to_string()
    })?;
    let repo_name = repo.file_name().ok_or_else(|| {
        "create_terminal_worktree: repository path has no directory name".to_string()
    })?;
    Ok(parent.join(format!(
        ".{}.demeteo-terminal-worktrees",
        repo_name.to_string_lossy()
    )))
}

/// Build the one target-machine transaction used to prepare a terminal
/// worktree destination and add it to Git.
///
/// The worktree area and every requested parent are created one component at
/// a time. `mkdir -p` is deliberately not used: it silently follows a
/// symlink. Starting at the repository's retained sibling directory, this
/// flow checks and then enters every component relative to its current working
/// directory. It also compares the physical directory after every `cd` with
/// the expected child. Once the final parent is entered, Git receives only the
/// destination basename, so a later rename-and-symlink substitution cannot
/// redirect it by causing another pathname resolution of that parent.
fn terminal_worktree_create_cmd(
    repo_dir: &str,
    branch: &str,
    destination: &str,
) -> Result<String, String> {
    let destination_path = Path::new(destination);
    let area = terminal_worktree_area(repo_dir)?;
    let parent = destination_path.parent().ok_or_else(|| {
        "create_terminal_worktree: destination has no parent directory".to_string()
    })?;
    let area_parent = area.parent().ok_or_else(|| {
        "create_terminal_worktree: controlled area has no parent directory".to_string()
    })?;

    let mut directories: Vec<&Path> = parent
        .ancestors()
        .take_while(|candidate| *candidate != area_parent)
        .collect();
    directories.reverse();
    if directories.first().copied() != Some(area.as_path()) {
        return Err("create_terminal_worktree: destination escaped controlled area".to_string());
    }

    let trusted_parent = paths::shell_escape_posix(&area_parent.to_string_lossy());
    let prepare = directories
        .iter()
        .map(|directory| {
            let component = directory.file_name().ok_or_else(|| {
                "create_terminal_worktree: controlled parent has no directory name".to_string()
            })?;
            let component = paths::shell_escape_posix(&component.to_string_lossy());
            Ok(format!(
                "if [ -L ./{component} ]; then echo 'terminal worktree parent is a symlink' >&2; exit 1; fi; \\
                 if [ -e ./{component} ]; then [ -d ./{component} ] || {{ echo 'terminal worktree parent is not a directory' >&2; exit 1; }}; \\
                 else mkdir ./{component}; fi; \\
                 [ ! -L ./{component} ] && [ -d ./{component} ] || {{ echo 'terminal worktree parent changed during creation' >&2; exit 1; }}; \\
                 expected_child=\"${{expected_parent}}\"/{component}; cd ./{component}; actual_parent=$(pwd -P); \\
                 [ \"$actual_parent\" = \"$expected_child\" ] || {{ echo 'terminal worktree parent changed during creation' >&2; exit 1; }}; \\
                 expected_parent=$actual_parent"
            ))
        })
        .collect::<Result<Vec<_>, String>>()?
        .join("; ");
    let destination_name = destination_path
        .file_name()
        .ok_or_else(|| "create_terminal_worktree: destination has no directory name".to_string())?;
    let destination_name = paths::shell_escape_posix(&destination_name.to_string_lossy());
    let git_dir = paths::shell_escape_posix(&format!("{repo_dir}/.git"));
    let work_tree = paths::shell_escape_posix(repo_dir);
    let branch = paths::shell_escape_posix(branch);

    // This deterministic rendezvous exists only in the in-crate infrastructure
    // test. It lets that test replace the pathname after validation but before
    // Git is invoked, without adding a production hook or weakening the
    // transport-neutral ExecutionPort boundary.
    #[cfg(test)]
    let synchronization = "if [ -n \"${DEMETEO_TERMINAL_WORKTREE_TEST_READY:-}\" ]; then : > \"$DEMETEO_TERMINAL_WORKTREE_TEST_READY\"; while [ ! -e \"$DEMETEO_TERMINAL_WORKTREE_TEST_RELEASE\" ]; do sleep 0.01; done; fi; ";
    #[cfg(not(test))]
    let synchronization = "";

    Ok(format!(
        "set -eu; cd {trusted_parent}; expected_parent=$(pwd -P); {prepare}; \\
         {synchronization}if [ -e ./{destination_name} ] || [ -L ./{destination_name} ]; then echo 'terminal worktree destination already exists' >&2; exit 1; fi; \\
         git --git-dir={git_dir} --work-tree={work_tree} worktree add -b {branch} ./{destination_name}",
    ))
}

/// Validate the user branch before it is handed to Git. This is the
/// `check-ref-format --branch` safety subset: it rejects ambiguous refs and
/// command-like names while allowing ordinary slash-separated branch names.
fn validate_terminal_branch(branch: &str) -> Result<(), String> {
    let invalid = branch.is_empty()
        || branch == "@"
        || branch.starts_with('-')
        || branch.starts_with('/')
        || branch.ends_with('/')
        || branch.ends_with('.')
        || branch.contains("..")
        || branch.contains("@{")
        || branch.contains("//")
        || branch.chars().any(|character| {
            character.is_ascii_control()
                || character.is_whitespace()
                || matches!(character, '~' | '^' | ':' | '?' | '*' | '[' | '\\')
        })
        || branch
            .split('/')
            .any(|component| component.starts_with('.') || component.ends_with(".lock"));
    if invalid {
        return Err(format!(
            "create_terminal_worktree: branch '{branch}' is not a safe Git branch name"
        ));
    }
    Ok(())
}

/// Build the shell command that symlinks each entry in
/// [`paths::DEPENDENCY_CACHE_DIRS`] from `repo_dir` into `wt_dir`, but
/// only when the entry exists as a real path in `repo_dir` *and* git
/// considers it ignored there. The `check-ignore` gate is the safety
/// net: if a project genuinely tracks a directory that happens to share
/// one of these names (unusual, but possible), it will not be ignored,
/// so we leave the worktree's own (correct) checkout of it alone rather
/// than shadowing it with a symlink to a different branch's copy.
/// Give `wt_dir` a working dependency install, without letting it share one
/// with any *other* feature.
///
/// For each well-known cache dir present and gitignored in the primary
/// checkout: seed this feature's own cache root from the primary (once — the
/// feature's later steps reuse it), then symlink the worktree at it.
///
/// The seeding copy tries the cheap options first. On APFS (`cp -c`) and on
/// btrfs/xfs (`--reflink=auto`) this is a copy-on-write clone: near-instant and
/// near-free in disk. Elsewhere it degrades to a real copy, which is slower but
/// still correct. Deliberately *not* hardlinks: a tool that rewrites a file in
/// place would write straight through a hardlink into every other feature's
/// tree, which is the exact bug this replaces.
///
/// When the primary has no copy of a dir, nothing is linked and the harness
/// installs into the worktree directly — already isolated, so that is fine.
fn link_dependency_caches_cmd(repo_dir: &str, wt_dir: &str, cache_dir: &str) -> String {
    let repo = paths::shell_escape_posix(repo_dir);
    let wt = paths::shell_escape_posix(wt_dir);
    let cache = paths::shell_escape_posix(cache_dir);
    let dirs = paths::DEPENDENCY_CACHE_DIRS.join(" ");
    format!(
        "mkdir -p {cache} 2>/dev/null; \
         for d in {dirs}; do \
         if [ -e {repo}/\"$d\" ] && git -C {repo} check-ignore -q \"$d\" 2>/dev/null; then \
         if [ ! -e {cache}/\"$d\" ]; then \
         cp -cR {repo}/\"$d\" {cache}/\"$d\" 2>/dev/null \
         || cp -R --reflink=auto {repo}/\"$d\" {cache}/\"$d\" 2>/dev/null \
         || cp -R {repo}/\"$d\" {cache}/\"$d\" 2>/dev/null; \
         fi; \
         if [ -e {cache}/\"$d\" ] && [ ! -e {wt}/\"$d\" ]; then \
         ln -sfn {cache}/\"$d\" {wt}/\"$d\"; \
         fi; \
         fi; \
         done",
        dirs = dirs,
        repo = repo,
        wt = wt,
        cache = cache,
    )
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/worktree.rs"]
mod tests;
