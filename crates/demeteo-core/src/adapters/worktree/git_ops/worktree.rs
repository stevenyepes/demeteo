use super::{git_request, git_request_vec, GitOpsHelper};
use crate::domain::branch_listing::BranchOption;
use crate::domain::feature_origin::Refspec;
use crate::domain::models::WorktreeInfo;
use crate::paths;
use crate::ports::worktree_ops::{TerminalWorktreeCreated, TerminalWorktreeRequest};
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
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--abbrev-ref", "HEAD"]),
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
            .run_program(machine_str, worktree_list_request(repo_dir))
            .await?;

        Ok(crate::domain::worktree_listing::parse(&output).linked)
    }

    /// Create a linked worktree for an interactive terminal session.
    ///
    /// Unlike subtask provisioning, this path never removes an existing
    /// worktree, resets a branch, or falls back to an existing branch. `git
    /// worktree add -b` creates the requested branch at the start point
    /// [`GitOpsHelper::terminal_start_point`] resolved; an existing requested
    /// branch is rejected rather than reused. The caller owns both names, so a
    /// collision is an error the user must resolve rather than stale pipeline
    /// state to reclaim.
    pub async fn create_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        request: &TerminalWorktreeRequest,
    ) -> Result<TerminalWorktreeCreated, String> {
        let branch = request.branch.as_str();
        validate_terminal_branch(branch)?;
        let destination = terminal_worktree_dir(repo_dir, project_root, &request.worktree_name)?;
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let base_ref = self
            .terminal_start_point(machine_str, repo_dir, request.base_branch.as_deref())
            .await?;
        let start_point = request.base_branch.as_ref().map(|_| base_ref.as_str());
        let command = terminal_worktree_create_cmd(
            repo_dir,
            project_root,
            branch,
            &destination,
            start_point,
            "",
        )?;
        let output = self
            .exec
            .run_command(machine_str, &command)
            .await
            .map_err(|e| {
                format!(
                    "create_terminal_worktree: git worktree add for branch '{branch}' failed: {e}"
                )
            })?;

        // The path the command resolved, not the one derived from
        // configuration. Git records the physical destination and replays it
        // from `worktree list`, so returning the logical form would hand the
        // caller a path that never equals the one it lists back — on macOS
        // `/var` and `/private/var` name the same directory and compare unequal.
        Ok(TerminalWorktreeCreated {
            worktree: WorktreeInfo {
                path: created_terminal_worktree_path(
                    &output,
                    &destination,
                    paths::targets_windows_host(machine_str),
                ),
                branch: Some(branch.to_string()),
                is_locked: false,
            },
            base_ref,
        })
    }

    /// Resolve what `git worktree add -b` should branch from, refreshing it
    /// from origin first.
    ///
    /// `None` answers `HEAD` without touching the network — the caller then
    /// omits the start point entirely and Git uses the primary checkout's HEAD,
    /// which is the pre-base-selection behaviour.
    ///
    /// With a base, the fetch is best-effort and the *probe after it* decides:
    /// `origin/<base>` when that ref resolves, the local `<base>` when only it
    /// does. So an unreachable origin degrades to a named local ref rather than
    /// to whatever the primary checkout is sitting on, and the caller learns
    /// which of the two it got. A base that resolves neither way is an error —
    /// silently falling through to HEAD is how a session starts on a branch
    /// nobody chose.
    pub(super) async fn terminal_start_point(
        &self,
        machine_str: &str,
        repo_dir: &str,
        base_branch: Option<&str>,
    ) -> Result<String, String> {
        let Some(base) = base_branch else {
            return Ok("HEAD".to_string());
        };
        validate_git_branch_name(base).map_err(|_| {
            format!("create_terminal_worktree: base branch '{base}' is not a safe Git branch name")
        })?;
        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["fetch", "origin", base]),
            )
            .await;

        for (reference, start_point) in [
            (
                format!("refs/remotes/origin/{base}"),
                format!("origin/{base}"),
            ),
            (format!("refs/heads/{base}"), base.to_string()),
        ] {
            if self
                .exec
                .run_program(
                    machine_str,
                    git_request(repo_dir, ["rev-parse", "--verify", "--quiet", &reference]),
                )
                .await
                .is_ok()
            {
                return Ok(start_point);
            }
        }

        Err(format!(
            "create_terminal_worktree: base branch '{base}' exists neither on origin nor locally"
        ))
    }

    /// Remove one terminal worktree, having first proved it is one.
    ///
    /// The path is re-derived, never trusted: the listing is taken again here
    /// and the request has to name something in it. A UI holding a stale list,
    /// or any caller at all, therefore cannot aim this at the primary checkout
    /// or at the worktree a pipeline step is mid-run in — the two directories
    /// `git worktree remove --force` would take with it silently.
    ///
    /// The prune afterwards is what makes the name reusable: a `remove` that
    /// leaves an administrative entry behind fails the next `add` of the same
    /// destination with "already used by worktree".
    pub async fn remove_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let windows_host = paths::targets_windows_host(machine_str);
        let owned = self
            .list_terminal_worktrees(machine_id, repo_dir, project_root)
            .await?;
        // One side of this is git's spelling and the other is whatever the
        // caller was handed; on Windows those are rarely the same string for
        // the same directory, which is what `paths::same_path` is for.
        let target = owned
            .into_iter()
            .find(|worktree| paths::same_path(&worktree.path, worktree_path, windows_host))
            .ok_or_else(|| {
                format!(
                    "remove_terminal_worktree: {worktree_path} is not a terminal worktree of {repo_dir}"
                )
            })?;

        let mut args = vec!["worktree".to_string(), "remove".to_string()];
        if force {
            args.push("--force".to_string());
        }
        args.push(target.path);
        self.exec
            .run_program(machine_str, git_request_vec(repo_dir, args))
            .await
            .map_err(|e| format!("remove_terminal_worktree: git worktree remove failed: {e}"))?;
        let _ = self
            .exec
            .run_program(machine_str, git_request(repo_dir, ["worktree", "prune"]))
            .await;

        Ok(())
    }

    /// Restate a terminal-worktree failure when the repository it names is not
    /// checked out at all.
    ///
    /// Cloning belongs to `application::bootstrap` and runs when a project is
    /// set up; every operation here assumes it already has. When it has not — a
    /// workspace cleared out from under a project, a moved `workspace_base_dir`
    /// — Git answers in its own terms, `fatal: cannot change to '<dir>'`, which
    /// reads as a damaged repository rather than one that was never cloned and
    /// names nothing the user can act on. Choosing a base branch makes it worse:
    /// [`GitOpsHelper::terminal_start_point`] probes first and blames the base
    /// for a directory that is not there.
    ///
    /// Probed only after a failure, so the extra round trip never lands on a
    /// call that worked — the one that counts over SSH.
    pub(super) async fn explain_missing_checkout(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        error: String,
    ) -> String {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let checked_out = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--is-inside-work-tree"]),
            )
            .await
            .is_ok();
        if checked_out {
            return error;
        }
        format!(
            "{repo_dir} is not a Git checkout — this project's repository has not been cloned \
             there. Re-run Bootstrap from the project's Settings › Workspace Health, then try \
             again. ({error})"
        )
    }

    /// The branches this repository can cut a terminal worktree from.
    ///
    /// Full ref names, not `%(refname:short)`: shortening is what makes
    /// `origin/main` and a local branch someone literally named `origin/main`
    /// arrive as the same string, and [`crate::domain::branch_listing`] tells
    /// the two apart by prefix.
    pub async fn list_terminal_branches(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<BranchOption>, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let output = self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "for-each-ref",
                        "--format=%(refname)",
                        "refs/heads",
                        "refs/remotes/origin",
                    ],
                ),
            )
            .await?;

        Ok(crate::domain::branch_listing::parse(&output))
    }

    /// The linked worktrees a terminal session may be opened in.
    ///
    /// One listing serves both halves: the primary checkout Git names first is
    /// the physical anchor
    /// [`domain::terminal_worktree::selectable`](crate::domain::terminal_worktree::selectable)
    /// needs, and the rest are the candidates. It never leaves this function.
    pub async fn list_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let output = self
            .exec
            .run_program(machine_str, worktree_list_request(repo_dir))
            .await?;
        let listing = crate::domain::worktree_listing::parse(&output);
        let primary = listing.primary.ok_or_else(|| {
            format!("list_terminal_worktrees: git reported no primary worktree for {repo_dir}")
        })?;

        crate::domain::terminal_worktree::selectable(
            project_root,
            repo_dir,
            &primary.path,
            listing.linked,
        )
        .ok_or_else(|| {
            format!(
                "list_terminal_worktrees: no terminal area for repository {repo_dir} below \
                 project root {project_root} (git reports the checkout at {})",
                primary.path
            )
        })
    }

    /// Retire terminal worktrees still registered at the pre-relocation
    /// location, returning how many were unregistered.
    ///
    /// See [`terminal_worktree_area`] for why that location was abandoned. The
    /// order matters and mirrors [`GitOpsHelper::cleanup_subtask_worktree`]:
    /// `worktree remove` first so Git forgets the entry, then `prune` for any
    /// whose directory a previous reclaim already deleted, and only then the
    /// area itself.
    pub async fn cleanup_legacy_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<usize, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let Some(area) = legacy_terminal_worktree_area(repo_dir) else {
            return Ok(0);
        };
        let marker = area
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let stale: Vec<String> = self
            .list_worktrees(machine_id, repo_dir)
            .await?
            .into_iter()
            .filter(|worktree| has_path_component(&worktree.path, &marker))
            .map(|worktree| worktree.path)
            .collect();

        for path in &stale {
            let _ = self
                .exec
                .run_program(
                    machine_str,
                    git_request(repo_dir, ["worktree", "remove", "--force", path]),
                )
                .await;
        }

        let _ = self
            .exec
            .run_program(machine_str, git_request(repo_dir, ["worktree", "prune"]))
            .await;
        let _ =
            delete_worktree_residue(self.exec.as_ref(), machine_str, &area.to_string_lossy()).await;

        Ok(stale.len())
    }

    /// Fetch one refspec from origin. See
    /// [`WorktreeOpsPort::fetch_origin_refspec`](crate::ports::worktree_ops::WorktreeOpsPort::fetch_origin_refspec)
    /// for why this one is not best-effort like its neighbours.
    pub async fn fetch_origin_refspec(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        refspec: &Refspec,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let spec = refspec.as_str();
        self.exec
            .run_program(
                machine_str,
                // Never drop the `--`: without it git reads a refspec
                // beginning with `-` as an option. See `Refspec`.
                git_request(repo_dir, ["fetch", "origin", "--", spec]),
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to fetch '{spec}' from origin: {e}"))
    }

    /// Point a branch at an already-resolvable start point, with no fallback.
    /// See
    /// [`WorktreeOpsPort::cut_branch_at`](crate::ports::worktree_ops::WorktreeOpsPort::cut_branch_at).
    pub async fn cut_branch_at(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        start_point: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        self.exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["branch", "-f", branch_name, start_point]),
            )
            .await
            .map(|_| ())
            .map_err(|e| format!("Failed to create branch '{branch_name}' at '{start_point}': {e}"))
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
        let tracking = format!("origin/{default_branch}");
        let branch_ref = format!("refs/heads/{branch_name}");

        // Create/update the feature branch ref without checking it out.
        // `git branch -f <branch> <start>` is a ref-only operation — it never
        // moves HEAD, so the main repo stays on the default branch throughout
        // the entire pipeline run. All agent work happens in linked worktrees;
        // the main checkout must not be disturbed.
        //
        // Prefer `origin/<default>` as the start point (latest upstream);
        // fall back to the local `<default>` when there's no remote-tracking
        // ref (offline, no remote, brand-new repo).
        if self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["branch", "-f", branch_name, &tracking]),
            )
            .await
            .is_ok()
        {
            return Ok(());
        }

        match self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["branch", "-f", branch_name, default_branch]),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(create_err) => {
                // Branch may already exist from a prior interrupted run.
                // Verify the ref is reachable; if so, we can proceed.
                self.exec
                    .run_program(
                        machine_str,
                        git_request(repo_dir, ["rev-parse", "--verify", &branch_ref]),
                    )
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
    /// Robust against the "already exists" failure mode: an interrupted run
    /// can leave a registered worktree in `.git/worktrees/`, an orphan
    /// directory with clean git metadata, stale branch metadata, or a scope
    /// fence that blocks the removal of all three, and `reclaim_worktree_path`
    /// takes them off in the one order that works. `git worktree add --force`
    /// is the final safety net after it.
    pub async fn provision_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<String, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt_dir = worktree_dir_on(repo_dir, subtask_id, machine_str);
        let subtask_branch = super::subtask_branch_name(feature_branch, subtask_id);

        // 1. Whatever an interrupted run left at this path — a registered
        //    worktree, an orphan directory, stale metadata, a fence that
        //    blocks all three — comes off in one ordered pass. The error is
        //    propagated because the path is about to be created: silently
        //    continuing produced the bug where `git worktree add` failed with
        //    "'<path>' already exists" and the user had no idea why.
        self.clear_worktree_path(machine_str, repo_dir, &wt_dir)
            .await
            .map_err(|e| format!("provision_subtask_worktree: {e}"))?;

        // 2. Create the worktree. `--force` lets git overwrite any
        //    remaining state (e.g. a missing-but-registered dir) so
        //    this last step is the safety net.
        match self
            .exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    [
                        "worktree",
                        "add",
                        "--force",
                        &wt_dir,
                        "-b",
                        &subtask_branch,
                        feature_branch,
                    ],
                ),
            )
            .await
        {
            Ok(_) => {}
            Err(_) => {
                // Fallback: the branch already exists from a prior interrupted
                // run, so `-b` refused. Check it out without `-b`.
                self.exec
                    .run_program(
                        machine_str,
                        git_request(
                            repo_dir,
                            ["worktree", "add", "--force", &wt_dir, &subtask_branch],
                        ),
                    )
                    .await?;

                // That branch still points at the *previous* attempt's tip —
                // it carries commits the failed attempt made, and the caller
                // has since reset the feature branch away from them. Left as
                // is, the new attempt would build on abandoned work and merge
                // all of it back, silently defeating the rollback. Provisioning
                // must hand back a worktree at the feature branch, exactly as
                // the `-b` path does.
                self.exec
                    .run_program(
                        machine_str,
                        git_request(&wt_dir, ["reset", "--hard", feature_branch]),
                    )
                    .await?;
            }
        }

        share_dependency_caches(
            self.exec.as_ref(),
            machine_str,
            repo_dir,
            &wt_dir,
            &paths::feature_cache_dir(repo_dir, feature_branch),
            paths::targets_windows_host(machine_str),
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
            .run_program(machine_str, git_request(dir, ["rev-parse", "HEAD"]))
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
        let wt_dir = worktree_dir_on(repo_dir, worktree_id, machine_str);

        self.clear_worktree_path(machine_str, repo_dir, &wt_dir)
            .await
            .map_err(|e| format!("provision_detached_worktree: {e}"))?;

        self.exec
            .run_program(
                machine_str,
                git_request(
                    repo_dir,
                    ["worktree", "add", "--detach", "--force", &wt_dir, sha],
                ),
            )
            .await
            .map_err(|e| {
                format!(
                    "provision_detached_worktree: git worktree add --detach at {sha} failed: {e}"
                )
            })?;

        if let Some(cache) = cache_dir {
            share_dependency_caches(
                self.exec.as_ref(),
                machine_str,
                repo_dir,
                &wt_dir,
                cache,
                paths::targets_windows_host(machine_str),
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
        let wt_dir = worktree_dir_on(repo_dir, worktree_id, machine_str);
        let _ = self
            .clear_worktree_path(machine_str, repo_dir, &wt_dir)
            .await;
        Ok(())
    }

    /// Get a worktree path back to "nothing is here": restore the write access
    /// the artifact-scope fence stripped, deregister it with git, prune the
    /// administrative entry, and delete whatever is still on disk.
    ///
    /// Only the final delete propagates its error — the rest are best-effort
    /// for the reasons `provision_subtask_worktree` documents at length (each
    /// handles a *leftover* state that usually isn't there). A failed delete is
    /// different: it means the path is still occupied, so whatever the caller
    /// was about to create there cannot be created.
    async fn clear_worktree_path(
        &self,
        machine_str: &str,
        repo_dir: &str,
        wt_dir: &str,
    ) -> Result<(), String> {
        let _ = self
            .restore_artifact_scope(
                if machine_str == crate::domain::ids::LOCAL_MACHINE {
                    None
                } else {
                    Some(machine_str)
                },
                wt_dir,
            )
            .await;
        reclaim_worktree_path(self.exec.as_ref(), machine_str, repo_dir, wt_dir).await
    }

    /// Clean up a linked worktree for a subtask, including its branch.
    ///
    /// The branch delete comes last and is unconditional: a branch still
    /// checked out in a worktree cannot be deleted, so it has to follow the
    /// removal, and a detached-or-already-gone branch makes it a harmless
    /// no-op.
    ///
    /// # This returns `Err` and every caller ignores it
    ///
    /// Deliberately. A teardown runs on the step's success path as well as its
    /// failure path and must never change the step's own outcome, so the
    /// callers are right to drop it — but the directory that survived, and the
    /// reason it did, are the two things
    /// [`WorktreeCleanupQueuePort`](crate::ports::worktree_cleanup::WorktreeCleanupQueuePort)
    /// needs to retry it at the next start, and losing them here would leave
    /// the leak with no record anywhere. The queue is not reachable from this
    /// type yet; until it is, the `Err` and the log line are the record.
    pub async fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let wt_dir = worktree_dir_on(repo_dir, subtask_id, machine_str);
        let subtask_branch = super::subtask_branch_name(feature_branch, subtask_id);

        let cleared = self
            .clear_worktree_path(machine_str, repo_dir, &wt_dir)
            .await;

        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["branch", "-D", &subtask_branch]),
            )
            .await;

        if let Err(error) = &cleared {
            tracing::error!(
                machine = %machine_str,
                worktree = %wt_dir,
                error = %error,
                "cleanup_subtask_worktree: worktree survived teardown",
            );
        }
        cleared
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
        let prefix = format!("{branch}{}", crate::domain::ids::SUBTASK_BRANCH_INFIX);
        if let Ok(worktrees) = self.list_worktrees(machine_id, repo_dir).await {
            for wt in &worktrees {
                let is_match = wt.branch.as_deref().is_some_and(|b| b.starts_with(&prefix));
                if is_match {
                    let _ = self
                        .clear_worktree_path(machine_str, repo_dir, &wt.path)
                        .await;
                }
            }
        }

        // 2. Prune orphaned worktree metadata so the refs are no longer
        //    considered "checked out" by the branch deletes below.
        let _ = self
            .exec
            .run_program(machine_str, git_request(repo_dir, ["worktree", "prune"]))
            .await;

        // 3. Delete all subtask branches for this feature (now that their
        //    worktrees are gone). Use `--format=%(refname:short)` so the
        //    listing emits clean branch names — `git branch --list` prefixes
        //    each line with two spaces (or `* `), and `IFS= read` preserves
        //    that leading whitespace, so `git branch -D "  <name>"` would
        //    never match a real ref.
        let subtask_cmd = format!(
            "git -C {} branch --list '{}{}*' --format='%(refname:short)' | while IFS= read -r b; do git -C {} branch -D \"$b\" 2>/dev/null; done",
            safe_dir,
            safe_branch,
            crate::domain::ids::SUBTASK_BRANCH_INFIX,
            safe_dir
        );
        let _ = self.exec.run_command(machine_str, &subtask_cmd).await;

        // 4. Drop this feature's dependency-cache root. It is per-feature (see
        //    `paths::feature_cache_dir`), so nothing else can be using it once
        //    the feature is gone — and it holds a whole `node_modules` /
        //    `target`, which would otherwise leak once per feature, forever.
        let _ = delete_worktree_residue(
            self.exec.as_ref(),
            machine_str,
            &paths::feature_cache_dir(repo_dir, branch),
        )
        .await;

        // 5. Delete the feature branch itself.
        self.exec
            .run_program(machine_str, git_request(repo_dir, ["branch", "-D", branch]))
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
        // git rev-parse HEAD gives the current tip; compare it to the stored baseline SHA.
        let Ok(current_sha) = self
            .exec
            .run_program(machine_str, git_request(target_dir, ["rev-parse", "HEAD"]))
            .await
        else {
            // git failure → assume something happened, allow validate.
            return true;
        };
        current_sha.trim() != base.trim()
    }

    /// Resolve the commit where `branch` most recently diverged from
    /// `base_branch` — the feature's fork point. Used to compute a
    /// review diff that always covers the complete feature change,
    /// independent of how many `on_failure` retries have merged work
    /// back into `branch` since (a per-attempt base SHA, recaptured as
    /// `branch`'s current tip on each retry, already includes prior
    /// attempts' merged commits and so understates the diff).
    ///
    /// `refs/remotes/origin/<base_branch>` is tried before the bare name, the
    /// same order [`squash_feature_branch`](Self::squash_feature_branch)
    /// uses: a run's base is often a branch this clone has never checked out,
    /// so the bare name resolves to nothing and the whole review degrades to
    /// "orient yourself from the log" while still reading as finished. The
    /// bare name remains as the fallback for a repo with no origin (tests,
    /// air-gapped clones).
    ///
    /// Returns `None` if neither candidate resolves or `git merge-base`
    /// fails (e.g. the two branches share no history) — callers fall
    /// back to their pre-existing per-attempt base.
    pub async fn merge_base(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        base_branch: &str,
        branch: &str,
    ) -> Option<String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let remote = format!("refs/remotes/origin/{base_branch}");
        for candidate in [remote.as_str(), base_branch] {
            let resolved = self
                .exec
                .run_program(
                    machine_str,
                    git_request(repo_dir, ["merge-base", candidate, branch]),
                )
                .await
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if resolved.is_some() {
                return resolved;
            }
        }
        None
    }

    /// [`merge_base`](Self::merge_base) against a base this clone may never
    /// have fetched.
    ///
    /// The bootstrap's
    /// [`ensure_default_branch_updated`](Self::ensure_default_branch_updated)
    /// fetches exactly one branch by name — the project's default — so a run
    /// whose base is anything else has no fresh `origin/<base>` to measure
    /// from, and on a fresh clone no `origin/<base>` at all. The fetch is
    /// best-effort for the reason the squash's is: an unreachable origin
    /// should degrade to whatever refs are local, not fail the review.
    pub async fn fork_point(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        base_branch: &str,
        branch: &str,
    ) -> Option<String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
        let _ = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["fetch", "origin", base_branch]),
            )
            .await;
        self.merge_base(machine_id, repo_dir, base_branch, branch)
            .await
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
///
/// # `shorten`, and the limit it is actually about
///
/// `worktree_id` is `{feature_id}-step-{step_id}` — around 32 characters before
/// a repo-relative path exists at all. On Windows the binding ceiling is not
/// the filesystem: `std::fs` goes through `maybe_verbatim` and is long-path
/// safe, but `std`'s process spawning strips the `\\?\` prefix before handing
/// `lpCurrentDirectory` to `CreateProcessW`, which fails past `MAX_PATH`
/// **whatever `core.longpaths` or the registry key say**. The agent spawn
/// therefore dies before `node_modules` does, and shortening the segment is the
/// only thing that moves that limit.
///
/// Only this segment is shortened, and only for a Windows-local target
/// ([`paths::windows_host_target`]). The clone's own path
/// (`projects/<project_id>/repos/<name>`) is left exactly as it is on every
/// platform: **nothing persists it**, so bootstrap, the workspace health check
/// and the step executor all re-derive it from `workspace_dir` and the
/// `projects.id` column — change the derivation and every already-cloned
/// project on that host becomes "not cloned", including on Windows, where the
/// desktop app has shipped since before this branch. A worktree directory has
/// no such problem: it is ephemeral, re-derived and re-created by the step that
/// uses it, and `git worktree prune` retires whatever registration an older
/// spelling left behind.
fn worktree_dir(repo_dir: &str, worktree_id: &str, shorten: bool) -> String {
    if shorten {
        format!("{}_wt_{}", repo_dir, paths::short_path_segment(worktree_id))
    } else {
        format!("{}_wt_{}", repo_dir, worktree_id)
    }
}

/// [`worktree_dir`] for a target machine.
fn worktree_dir_on(repo_dir: &str, worktree_id: &str, machine_id: &str) -> String {
    worktree_dir(
        repo_dir,
        worktree_id,
        paths::targets_windows_host(machine_id),
    )
}

/// Take a worktree path apart, in the one order in which each step can
/// succeed.
///
/// The order **is** the content of this function, and every step of it exists
/// because the step before it would otherwise fail:
///
/// 1. **Restore write access.** `unlink()` needs write on the *parent*
///    directory, so an `a-w src/` left by the artifact-scope fence blocks both
///    the `git worktree remove` below and the delete after it, each of which
///    then fails partway through and leaves a gutted directory skeleton.
/// 2. **`git worktree remove --force --force`.** The doubled force is not a
///    typo: a single `-f` **refuses a locked worktree**, and a lock is exactly
///    what a crashed or killed step leaves behind — so single-`-f` fails
///    precisely when teardown matters. `-f -f` also covers the dirty-*and*-
///    locked worktree, which is the same run.
/// 3. **`git worktree prune`.** What makes the *name* reusable: a `remove` that
///    left an administrative entry behind fails the next `add` of the same
///    destination with "already used by worktree". This runs whether or not
///    the remove worked, since the remove failing is when there is an entry to
///    prune.
/// 4. **Delete the residue.** Git leaves the directory behind when it declines
///    to remove it, and a directory the fence or a still-running descendant
///    held is the whole reason this is retried at all.
///
/// The one thing this cannot do is drop the step's process guard first: a
/// descendant still holding a handle keeps the directory undeletable on
/// Windows, and that guard belongs to the step executor, not here.
pub(crate) async fn reclaim_worktree_path(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    wt_dir: &str,
) -> Result<(), String> {
    if let Some(command) = restore_write_access_cmd(wt_dir, paths::targets_windows_host(machine_id))
    {
        let _ = exec.run_command(machine_id, &command).await;
    }
    let _ = exec
        .run_program(
            machine_id,
            git_request(
                repo_dir,
                ["worktree", "remove", "--force", "--force", wt_dir],
            ),
        )
        .await;
    let _ = exec
        .run_program(machine_id, git_request(repo_dir, ["worktree", "prune"]))
        .await;
    delete_worktree_residue(exec, machine_id, wt_dir).await
}

/// Undo the Unix half of the artifact-scope fence.
///
/// `None` for a Windows-local target: there the fence is an ACL that
/// `GitOpsHelper::restore_artifact_scope` has already lifted, and a POSIX
/// `chmod` sent to Git Bash would walk the whole tree to change nothing.
fn restore_write_access_cmd(wt_dir: &str, windows_target: bool) -> Option<String> {
    (!windows_target).then(|| {
        format!(
            "chmod -R u+w {} 2>/dev/null || true",
            paths::shell_escape_posix(wt_dir)
        )
    })
}

/// Delete what git left behind, and decide whether a failure to do so matters.
///
/// [`ExecutionPort::remove_dir_all`](crate::ports::execution::ExecutionPort::remove_dir_all)
/// reports an absent path as an error — deliberately, so the contract reads the
/// same over SFTP as it does locally — and a teardown reaches this with the
/// directory already gone often enough that the error alone cannot be the
/// verdict. Rather than matching on the message, the path is asked about
/// directly: still there is a failure, gone is done, whatever the delete said.
///
/// The message names the path first so a
/// [`LeakedWorktree`](crate::ports::worktree_cleanup::LeakedWorktree) row and
/// the log line that produced it can be lined up without parsing it.
pub(crate) async fn delete_worktree_residue(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    wt_dir: &str,
) -> Result<(), String> {
    let Err(error) = exec.remove_dir_all(machine_id, wt_dir).await else {
        return Ok(());
    };
    if exec.get_metadata(machine_id, wt_dir).await.is_err() {
        return Ok(());
    }
    Err(format!(
        "{wt_dir} could not be deleted: {error}. Something still holds it open, or it is \
         owned by another user; deleting it by hand is what frees the name."
    ))
}

/// The one reading every worktree listing in this crate is built from. Shared
/// so the terminal listing and merge-back cannot end up asking git a
/// differently-shaped question than [`GitOpsHelper::list_worktrees`] does.
pub(super) fn worktree_list_request(dir: &str) -> crate::ports::execution::ProgramRequest {
    super::git_request(dir, ["worktree", "list", "--porcelain"])
}

/// Resolve a terminal worktree beneath a directory controlled by Demeteo, not
/// by the interactive caller. A relative name may contain normal path
/// components, but it may never select the repository root, an absolute path,
/// or an ancestor of that directory.
fn terminal_worktree_dir(
    repo_dir: &str,
    project_root: &str,
    worktree_name: &str,
) -> Result<String, String> {
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

    let worktree_area = terminal_worktree_area(repo_dir, project_root)?;
    let destination: PathBuf = worktree_area.join(name);
    Ok(destination.to_string_lossy().into_owned())
}

/// `<project_root>/terminal-worktrees/<repo_name>` — a sibling of the project's
/// `repos/`, never a child of it.
///
/// These worktrees hold the user's uncommitted interactive work, and
/// `application::bootstrap` prunes `repos/` down to the configured repository
/// names on every re-bootstrap. Inside `repos/` the area is destroyed, and
/// destroyed *asymmetrically*: the local prune walks `read_dir`, which yields
/// dot-entries, while the remote prune globs `"$dir"/*`, which does not match a
/// leading dot — so a hidden sibling of the checkout survived remotely and
/// vanished locally. Keeping the area out of `repos/` is what makes the two
/// transports agree.
fn terminal_worktree_area(repo_dir: &str, project_root: &str) -> Result<PathBuf, String> {
    let repo_name = Path::new(repo_dir).file_name().ok_or_else(|| {
        "create_terminal_worktree: repository path has no directory name".to_string()
    })?;
    Ok(Path::new(project_root)
        .join(paths::TERMINAL_WORKTREES_SUBDIR)
        .join(repo_name))
}

/// `<repos>/.<repo_name>.demeteo-terminal-worktrees` — where terminal
/// worktrees lived before [`paths::TERMINAL_WORKTREES_SUBDIR`] moved them out
/// of `repos/`. Retained only so
/// [`GitOpsHelper::cleanup_legacy_terminal_worktrees`] can find and retire
/// them; nothing creates this path any more.
fn legacy_terminal_worktree_area(repo_dir: &str) -> Option<PathBuf> {
    let repo = Path::new(repo_dir);
    let parent = repo.parent()?;
    let repo_name = repo.file_name()?;
    Some(parent.join(format!(
        ".{}.demeteo-terminal-worktrees",
        repo_name.to_string_lossy()
    )))
}

/// Match a directory name anywhere in `path`, rather than comparing a prefix
/// against a computed root: Git replays the path it resolved when the worktree
/// was added, which is physical, while a path built from configuration is
/// logical, and on macOS those differ (`/var` → `/private/var`).
fn has_path_component(path: &str, name: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component.as_os_str() == name)
}

/// Build the one target-machine transaction used to prepare a terminal
/// worktree destination and add it to Git.
///
/// The worktree area and every requested parent are created one component at
/// a time. `mkdir -p` is deliberately not used: it silently follows a
/// symlink. Starting at the project root — the one directory here that Demeteo
/// owns and that already exists — this flow checks and then enters every
/// component relative to its current working directory, including
/// `terminal-worktrees` and the per-repository directory below it. It also
/// compares the physical directory after every `cd` with the expected child.
/// Once the final parent is entered, Git receives only the destination
/// basename, so a later rename-and-symlink substitution cannot redirect it by
/// causing another pathname resolution of that parent.
///
/// `start_point` is the committish the new branch is cut at, already resolved
/// by [`GitOpsHelper::terminal_start_point`]. `None` omits it so Git falls back
/// to the primary checkout's HEAD.
///
/// `interlude` is shell run after the destination parent has been entered and
/// before Git is invoked. Production passes `""`; only a test that has to act
/// inside that check-to-use window passes anything else, so the string this
/// builds is otherwise the same one production emits.
fn terminal_worktree_create_cmd(
    repo_dir: &str,
    project_root: &str,
    branch: &str,
    destination: &str,
    start_point: Option<&str>,
    interlude: &str,
) -> Result<String, String> {
    let destination_path = Path::new(destination);
    let area = terminal_worktree_area(repo_dir, project_root)?;
    let parent = destination_path.parent().ok_or_else(|| {
        "create_terminal_worktree: destination has no parent directory".to_string()
    })?;
    let trusted_root = Path::new(project_root);
    let area_root = trusted_root.join(paths::TERMINAL_WORKTREES_SUBDIR);

    let mut directories: Vec<&Path> = parent
        .ancestors()
        .take_while(|candidate| *candidate != trusted_root)
        .collect();
    directories.reverse();
    if directories.first().copied() != Some(area_root.as_path())
        || !destination_path.starts_with(&area)
        || destination_path == area
    {
        return Err("create_terminal_worktree: destination escaped controlled area".to_string());
    }

    let trusted_parent = paths::shell_escape_posix(project_root);
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
    let work_tree = paths::shell_escape_posix(repo_dir);
    let branch = paths::shell_escape_posix(branch);
    let start_point = start_point
        .map(|start| format!(" {}", paths::shell_escape_posix(start)))
        .unwrap_or_default();

    // `rev-parse` rather than a literal `<repo_dir>/.git`. Not because the
    // literal is known to break — Git resolves a `.git` *file* handed to
    // `--git-dir`, so submodule and linked-worktree checkouts work either way —
    // but because it is the only place in this module that asserts where a
    // repository keeps its git directory. Every other call here uses `git -C`
    // and gets discovery for free; this one cannot, because the shell has to
    // stay in the destination parent it just checked.
    Ok(format!(
        "set -eu; git_dir=$(git -C {work_tree} rev-parse --absolute-git-dir); \\
         cd {trusted_parent}; expected_parent=$(pwd -P); {prepare}; \\
         {interlude}if [ -e ./{destination_name} ] || [ -L ./{destination_name} ]; then echo 'terminal worktree destination already exists' >&2; exit 1; fi; \\
         git --git-dir=\"$git_dir\" --work-tree={work_tree} worktree add -b {branch} ./{destination_name}{start_point}; \\
         printf '%s\\n' \"${{expected_parent}}\"/{destination_name}",
    ))
}

/// The physical destination the command reports on its last line.
///
/// Taking the *last* line rather than the whole output: `git worktree add`
/// writes its progress to stderr, but that is a convention rather than a
/// guarantee, and a transport is free to interleave. Falling back to the
/// derived path keeps a silent stdout from failing the whole create.
///
/// The line was printed by a shell, so on a Windows host it is `pwd`'s MSYS
/// spelling of the destination and not a path any Win32 call would accept — and
/// the caller opens a terminal at what this returns.
/// [`paths::native_path`] is where that spelling stops.
///
/// The absoluteness test belongs to the *target*, not to this build:
/// `Path::is_absolute` compiled for Windows rejects the `/srv/…` a Linux
/// machine reports, so a Windows desktop driving a remote would throw the
/// created path away and hand back its own guess on every create.
fn created_terminal_worktree_path(output: &str, derived: &str, windows_host: bool) -> String {
    output
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| paths::native_path(line, windows_host))
        .filter(|path| paths::is_absolute_on(&path.to_string_lossy(), windows_host))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|| derived.to_string())
}

/// Validate the branch a terminal worktree will be created *on*.
///
/// [`validate_git_branch_name`] plus a refusal of the pipeline's own subtask
/// infix, which Git would happily accept. `domain::terminal_worktree` withholds
/// any branch carrying it, so accepting one here would hand back a worktree that
/// then never appears in the listing again — created, on disk, and invisible,
/// with nothing said. A refusal at creation is the same rule, stated where the
/// user can act on it.
///
/// The infix is not refused for a *base* branch: reading a subtask branch is
/// harmless, and the listing already withholds them from the picker.
pub(super) fn validate_terminal_branch(branch: &str) -> Result<(), String> {
    if branch.contains(crate::domain::ids::SUBTASK_BRANCH_INFIX) {
        return Err(format!(
            "create_terminal_worktree: branch '{branch}' is not a safe Git branch name"
        ));
    }
    validate_git_branch_name(branch)
}

/// The `check-ref-format --branch` safety subset: it rejects ambiguous refs and
/// command-like names while allowing ordinary slash-separated branch names.
fn validate_git_branch_name(branch: &str) -> Result<(), String> {
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

/// Give `wt_dir` a working dependency install, without letting it share one
/// with any *other* feature — and without letting the mechanics of that
/// sharing reach a commit.
///
/// Three steps, in an order that is the point:
///
/// 1. **Ask the primary checkout which caches it has.** A name qualifies when
///    it exists there *and* git ignores it there. The ignore gate is the safety
///    net: a project that genuinely tracks a directory sharing one of these
///    names (Go's vendored `vendor/`) is left with its own correct checkout
///    rather than shadowed by another branch's copy. The answer comes from the
///    clone, not from the platform, so every host resolves the same set.
/// 2. **Record those names in the clone's `.git/info/exclude`.** See
///    [`exclude_file_with`] for why an exclude entry and not a `git add`
///    pathspec.
/// 3. **Link.** Only after step 2 has actually landed. A link whose exclusion
///    was not written puts an absolute host path in the feature branch, so a
///    failure at step 2 must degrade to *no sharing* — a slower harness — and
///    never to a committed symlink.
///
/// Step 3 is skipped for a Windows-local target. `ln -s` under Git for Windows
/// only produces a real link with Developer Mode or `SeCreateSymbolicLink`, and
/// silently copies otherwise, which would duplicate a whole `node_modules` per
/// step; a junction is worse, because git's walk follows its reparse tag
/// transparently, inverting the assumption steps 1–2 are built on. Cache
/// sharing is therefore **off on Windows** until that has a real answer — the
/// harness installs into the worktree, which is slower and already isolated.
/// Steps 1 and 2 still run there, so the exclusions a commit sees are the same
/// on every platform.
///
/// `windows_host` is the caller's [`paths::targets_windows_host`] answer, and
/// [`may_link_caches`] holds both conditions above it — neither the ordering
/// nor the platform arm is observable from the filesystem afterwards, so
/// neither is reachable from a test spelled inside this `async fn`.
///
/// Best-effort throughout: a failure here costs a re-install, not the step.
async fn share_dependency_caches(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    wt_dir: &str,
    cache_dir: &str,
    windows_host: bool,
) {
    let probed = exec
        .run_command(machine_id, &shareable_cache_probe_cmd(repo_dir))
        .await
        .unwrap_or_default();
    let names = shareable_cache_names(&probed);
    if names.is_empty() {
        return;
    }

    let excluded = record_cache_exclusions(exec, machine_id, repo_dir, &names).await;
    if let Err(error) = &excluded {
        tracing::warn!(
            machine = %machine_id,
            repo = %repo_dir,
            error = %error,
            "dependency caches are not shared into this worktree: the exclusion could not be written",
        );
    }

    if !may_link_caches(excluded.is_ok(), windows_host) {
        return;
    }
    let _ = exec
        .run_command(
            machine_id,
            &link_dependency_caches_cmd(repo_dir, wt_dir, cache_dir, &names),
        )
        .await;
}

/// Whether step 3 of [`share_dependency_caches`] may run.
///
/// Both answers are a commit-visible decision that a filesystem check after the
/// fact cannot tell apart from a slow install: linking without the exclusion
/// puts an absolute host path on the feature branch, and linking on Windows
/// copies a whole `node_modules` per step or — via a junction — hides one from
/// the walk steps 1–2 assume. So the conditions are here, where a Linux test
/// reaches both, rather than as two early returns in the pipeline.
fn may_link_caches(exclusions_written: bool, windows_host: bool) -> bool {
    exclusions_written && !windows_host
}

/// One round trip that prints the [`paths::DEPENDENCY_CACHE_DIRS`] entries the
/// primary checkout both has and ignores.
///
/// `; true` at the end for the reason `artifacts::add_exclusions` records: the
/// loop's exit status would otherwise be the last `if`'s, and a non-zero exit
/// makes `run_command` discard the whole answer.
fn shareable_cache_probe_cmd(repo_dir: &str) -> String {
    let repo = paths::shell_escape_posix(repo_dir);
    format!(
        "for d in {dirs}; do \
         if [ -e {repo}/\"$d\" ] && git -C {repo} check-ignore -q \"$d\" 2>/dev/null; then \
         echo \"$d\"; \
         fi; \
         done; true",
        dirs = paths::DEPENDENCY_CACHE_DIRS.join(" "),
        repo = repo,
    )
}

/// Read the probe's answer back as entries of [`paths::DEPENDENCY_CACHE_DIRS`]
/// rather than as strings.
///
/// The names go straight into a shell command and a git exclude file, and the
/// only thing standing between the transport's stdout and both of those is this
/// function. Matching against the constant means an unexpected line — a shell
/// banner, a git warning, anything a login profile prints — is dropped rather
/// than quoted, so no output can name a path the caller did not compile in.
fn shareable_cache_names(probe_output: &str) -> Vec<&'static str> {
    probe_output
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            paths::DEPENDENCY_CACHE_DIRS
                .iter()
                .find(|known| **known == line)
                .copied()
        })
        .collect()
}

/// Write the shared cache names into the clone's own `.git/info/exclude`.
///
/// `info/exclude` is per-*repository*, not per-worktree — git resolves it
/// through the common directory — so this is written once for the clone and
/// every linked worktree inherits it, which is the same shape
/// `git_ops::clone` uses for `core.autocrlf`.
async fn record_cache_exclusions(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
    names: &[&str],
) -> Result<(), String> {
    let git_dir = exec
        .run_program(
            machine_id,
            git_request(repo_dir, ["rev-parse", "--absolute-git-dir"]),
        )
        .await?;
    let exclude_path = target_path_join(git_dir.trim(), "info/exclude");
    let existing = exec
        .read_file(machine_id, &exclude_path)
        .await
        .unwrap_or_default();
    let Some(updated) = exclude_file_with(&existing, names) else {
        return Ok(());
    };
    exec.write_file(machine_id, &exclude_path, &updated).await
}

/// Append the entries `existing` does not already carry, or `None` when it
/// carries them all.
///
/// # Why an exclude entry rather than a `git add` pathspec
///
/// A symlink standing in for a directory is not matched by a trailing-slash
/// `.gitignore` pattern — `node_modules/` matches a real directory and not a
/// symlink named `node_modules` — so a linked cache reads as untracked and
/// `git add -A` stages an absolute host path onto the feature branch. That used
/// to be answered at `git add` time with `':!node_modules'`, which needs its own
/// gate: naming a path in a pathspec makes git treat it as explicitly
/// requested, so the same pathspec fails outright ("paths are ignored by one of
/// your .gitignore files") on the projects that *do* ignore it slashlessly.
///
/// The slashless exclude entry matches the symlink and the directory alike, so
/// there is nothing left for a pathspec to say and the gate disappears with it.
/// It also survives the platform question: a host that shares no caches links
/// nothing, and a worktree with a real installed `node_modules` and one with a
/// symlink to one then commit exactly the same files.
///
/// Names only, never a path: an entry is scoped to this clone's
/// `.git/info/exclude`, which is not committed, and appending is what keeps a
/// user's own entries in a repository Demeteo cloned but does not own the
/// contents of.
fn exclude_file_with(existing: &str, names: &[&str]) -> Option<String> {
    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !existing.lines().any(|line| line.trim() == *name))
        .collect();
    if missing.is_empty() {
        return None;
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(CACHE_EXCLUDE_HEADER);
    for name in missing {
        out.push('\n');
        out.push_str(name);
    }
    out.push('\n');
    Some(out)
}

/// What a human finds when they wonder who wrote to their exclude file.
const CACHE_EXCLUDE_HEADER: &str = "# demeteo: dependency caches shared into linked worktrees";

/// Join a path *on the target machine*, which is not necessarily this one.
///
/// [`std::path::Path::join`] uses the **host's** separator, so a Windows
/// desktop driving a Linux machine would build `…\info\exclude` and hand it to
/// SFTP. Forward slashes are accepted by Win32 and by git on every platform, so
/// one spelling serves both directions.
fn target_path_join(base: &str, relative: &str) -> String {
    format!("{}/{}", base.trim_end_matches(['/', '\\']), relative)
}

/// Seed this feature's cache root from the primary checkout, once, then point
/// the worktree at it.
///
/// The seeding copy tries the cheap options first. On APFS (`cp -c`) and on
/// btrfs/xfs (`--reflink=auto`) this is a copy-on-write clone: near-instant and
/// near-free in disk. Elsewhere it degrades to a real copy, which is slower but
/// still correct. Deliberately *not* hardlinks: a tool that rewrites a file in
/// place would write straight through a hardlink into every other feature's
/// tree, which is the exact bug this replaces.
fn link_dependency_caches_cmd(
    repo_dir: &str,
    wt_dir: &str,
    cache_dir: &str,
    names: &[&str],
) -> String {
    let repo = paths::shell_escape_posix(repo_dir);
    let wt = paths::shell_escape_posix(wt_dir);
    let cache = paths::shell_escape_posix(cache_dir);
    format!(
        "mkdir -p {cache} 2>/dev/null; \
         for d in {dirs}; do \
         if [ ! -e {cache}/\"$d\" ]; then \
         cp -cR {repo}/\"$d\" {cache}/\"$d\" 2>/dev/null \
         || cp -R --reflink=auto {repo}/\"$d\" {cache}/\"$d\" 2>/dev/null \
         || cp -R {repo}/\"$d\" {cache}/\"$d\" 2>/dev/null; \
         fi; \
         if [ -e {cache}/\"$d\" ] && [ ! -e {wt}/\"$d\" ]; then \
         ln -sfn {cache}/\"$d\" {wt}/\"$d\"; \
         fi; \
         done; true",
        dirs = names.join(" "),
        repo = repo,
        wt = wt,
        cache = cache,
    )
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/worktree.rs"]
mod tests;
