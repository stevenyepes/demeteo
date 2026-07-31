use super::GitOpsHelper;
use crate::paths;
use crate::ports::execution::ExecutionPort;
use crate::ports::worktree_ops::{SyncFailure, SyncOutcome};

impl GitOpsHelper {
    /// Fetch the latest state of `default_branch` from `origin` and update
    /// the local copy of that branch to match. This is the one-time
    /// "snapshot" call used at feature start so the user's local
    /// `<default>` ref doesn't lag behind upstream — which the validate
    /// step would otherwise read as "extra changes not in main" when
    /// comparing the freshly-cut feature branch (which IS based on
    /// `origin/<default>`) against a stale local ref.
    ///
    /// Idempotent and safe to re-invoke: it does not touch any feature
    /// branches, only the local `<default>` ref.
    ///
    /// On success, the local `<default>` ref matches `origin/<default>`.
    /// On a non-fatal fallback (HEAD on `<default>` with a dirty working
    /// tree), the function returns `Err` with a clear message that the
    /// caller surfaces as a soft bootstrap detail. The feature branch is
    /// still cut correctly in the next phase regardless, so the pipeline
    /// always proceeds.
    pub async fn ensure_default_branch_updated(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
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

        // 3. Update the local default-branch ref to match origin.
        //    Prefer the ref-only fast-forward (`git fetch origin +src:dst`),
        //    which never moves HEAD or the working tree. This works when
        //    HEAD is on any branch other than `<default>` — and the
        //    previous code relied on this path even when HEAD *was* on
        //    `<default>`, swallowing the inevitable Err ("refusing to
        //    fetch into current branch") and leaving the local ref stale.
        //
        //    The stale local ref was harmless to `create_feature_branch`
        //    (which cuts from `origin/<default>`), but the validate step
        //    in a subtask worktree would see `git log master..HEAD` as
        //    containing every commit upstream has merged since the user's
        //    last manual pull — "extra changes not in main". So when the
        //    ref-only fast-forward is rejected we fall back to a path
        //    that keeps the local ref in sync.
        let fetch_outcome = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} fetch origin +{}:{}",
                    safe_dir, safe_branch, safe_branch
                ),
            )
            .await;
        if fetch_outcome.is_ok() {
            return Ok(());
        }

        // Ref-only fast-forward was rejected (HEAD is on `<default>`,
        // or origin is unreachable — step 1 is best-effort). Try the
        // safe fallback that updates the local ref together with HEAD
        // and the working tree when the checkout state allows it.
        self.fast_forward_local_default_safe(machine_str, &safe_dir, default_branch, &tracking)
            .await
    }

    /// Fallback fast-forward of the local `<default>` ref when the
    /// ref-only `git fetch origin +src:dst` is rejected (git refuses to
    /// fetch into a checked-out branch). Branches on the local checkout
    /// state:
    ///
    /// - **HEAD on a different branch**: `git update-ref refs/heads/<default>
    ///   refs/remotes/origin/<default>`. Ref-only, safe because the
    ///   working tree belongs to a different branch — no HEAD, index, or
    ///   working-tree file gets out of sync.
    /// - **HEAD on `<default>` with a clean working tree**: `git merge
    ///   --ff-only origin/<default>`. Fast-forwards HEAD, the index, and
    ///   the working tree in one atomic step.
    /// - **HEAD on `<default>` with a dirty working tree**: return `Err`
    ///   with a clear message — the caller surfaces it as a soft
    ///   bootstrap detail ("local main is N commits behind; please
    ///   `git pull` manually"). The feature branch is still cut
    ///   correctly from `origin/<default>` in the next phase, so the
    ///   pipeline always proceeds.
    /// - **HEAD on `<default>` but local is ahead of origin** (or
    ///   diverged): `git merge --ff-only` rejects non-fast-forwards, so
    ///   the underlying git error is surfaced verbatim.
    async fn fast_forward_local_default_safe(
        &self,
        machine_str: &str,
        safe_dir: &str,
        default_branch: &str,
        tracking: &str,
    ) -> Result<(), String> {
        let head_branch = self
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse --abbrev-ref HEAD", safe_dir),
            )
            .await
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let safe_default = paths::shell_escape_posix(default_branch);
        let safe_tracking = paths::shell_escape_posix(tracking);

        if head_branch != default_branch {
            // HEAD is on a non-default branch (a feature branch the
            // user checked out to inspect, or a previously-cut feature
            // branch the previous run left behind). The working tree
            // doesn't claim to match `<default>`, so a ref-only update
            // is safe.
            return self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} update-ref refs/heads/{} {}",
                        safe_dir, safe_default, safe_tracking
                    ),
                )
                .await
                .map(|_| ())
                .map_err(|e| {
                    format!(
                        "Could not fast-forward local '{}' via update-ref: {}",
                        default_branch, e
                    )
                });
        }

        // HEAD is on `<default>`. A ref-only update would leave the
        // working tree and index out of sync with the new HEAD, which
        // is a real foot-gun (the user could `git add` + `git commit`
        // files that don't match the parent). Use the proper merge path
        // so HEAD, the index, and the working tree move together.
        let status_porcelain = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} status --porcelain --untracked-files=no",
                    safe_dir
                ),
            )
            .await
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        if !status_porcelain.is_empty() {
            let behind = self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} rev-list --count HEAD..{}",
                        safe_dir, safe_tracking
                    ),
                )
                .await
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .unwrap_or(0);
            return Err(format!(
                "Local '{}' is {} commit(s) behind origin/{} but the \
                 working tree has uncommitted changes; please `git pull` \
                 manually to keep the local default ref in sync.",
                default_branch, behind, default_branch
            ));
        }

        // Clean working tree, HEAD on `<default>` — fast-forward the
        // whole repo in one step. If the local branch has unpushed
        // commits (so origin isn't strictly ahead), `--ff-only` rejects
        // it and we surface the underlying error verbatim.
        self.exec
            .run_command(
                machine_str,
                &format!("git -C {} merge --ff-only {}", safe_dir, safe_tracking),
            )
            .await
            .map(|_| ())
            .map_err(|e| {
                format!(
                    "Local '{}' could not be fast-forwarded to origin/{}: {}. \
                     If the local branch has unpushed commits, pull with \
                     `--rebase` or merge manually.",
                    default_branch, default_branch, e
                )
            })
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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
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
                    "git -C {} merge {} -m \"chore(sync): sync feature with origin/{}\"",
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
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);

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

/// Ask `repo_dir` which files a merge left unresolved.
///
/// Shared by the sync flow and the existing `merge_subtask` conflict path so
/// both produce the same `ConflictFile` shape — the XY-code table itself lives
/// in [`crate::domain::merge_status`], which the step executor reads too.
pub(crate) async fn parse_unmerged_files(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<crate::domain::models::ConflictFile> {
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
    crate::domain::merge_status::parse_unmerged(&raw)
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/sync.rs"]
mod tests;
