use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use crate::paths;
use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::ExecutionPort;
use keyring::Entry;
use std::sync::Arc;

pub struct GitOpsHelper {
    app_settings: Arc<dyn AppSettingsRepository>,
    exec: Arc<dyn ExecutionPort>,
}

impl GitOpsHelper {
    pub fn new(app_settings: Arc<dyn AppSettingsRepository>, exec: Arc<dyn ExecutionPort>) -> Self {
        Self { app_settings, exec }
    }

    /// Retrieve the token for the given provider from Keyring (cached in-process).
    pub fn get_provider_pat(&self, provider_id: &str) -> Result<String, String> {
        crate::credential_cache::get_or_fetch(provider_id, || {
            let entry = Entry::new("demeteo", provider_id)
                .map_err(|e| format!("Failed to access keyring: {}", e))?;
            entry.get_password().map_err(|e| {
                format!(
                    "Token not found in keyring for provider '{}': {}",
                    provider_id, e
                )
            })
        })
    }

    /// Run clone operation. Clones to either local or remote path based on compute_type
    pub fn clone_repository(
        &self,
        machine_id: Option<&str>,
        provider_id: &str,
        repo_path: &str,
        target_dir: &str,
    ) -> Result<(), String> {
        // Resolve provider instance
        let providers = self.app_settings.get_provider_instances()?;
        let provider_id_typed = crate::domain::ids::ProviderId::from(provider_id.to_string());
        let provider = providers
            .into_iter()
            .find(|p| p.id == provider_id_typed)
            .ok_or_else(|| format!("Provider not found in DB: {}", provider_id))?;

        let pat = self.get_provider_pat(provider_id)?;

        // Construct the clone URL with credentials
        let clone_url = if provider.kind.to_lowercase() == "github" {
            format!(
                "https://x-access-token:{}@{}/{}",
                pat, provider.host, repo_path
            )
        } else {
            format!("https://oauth2:{}@{}/{}", pat, provider.host, repo_path)
        };

        // Ensure parent directory exists
        let machine_str = machine_id.unwrap_or("local");
        let path = std::path::Path::new(target_dir);
        if let Some(parent) = path.parent() {
            let parent_str = parent.to_str().unwrap_or("");
            self.exec.run_command(
                machine_str,
                &format!("mkdir -p {}", paths::shell_escape_posix(parent_str)),
            )?;
        }

        // Run clone
        let clone_cmd = format!(
            "git clone \"{}\" {}",
            clone_url,
            paths::shell_escape_posix(target_dir)
        );
        let output = self.exec.run_command(machine_str, &clone_cmd)?;
        println!("[GitOps] Clone output: {}", output);

        Ok(())
    }

    /// Run git analysis and propose strategy settings
    pub fn detect_worktree_strategy(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<WorktreeStrategy, String> {
        let machine_str = machine_id.unwrap_or("local");

        // 1. Detect Default Branch name
        // Try origin/HEAD first. Fallback to local HEAD, but reject feature/subtask branch names.
        let default_branch = match self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} rev-parse --abbrev-ref origin/HEAD",
                paths::shell_escape_posix(repo_dir)
            ),
        ) {
            Ok(out) => {
                let trimmed = out.trim().to_string();
                if let Some(stripped) = trimmed.strip_prefix("origin/") {
                    let branch = stripped.to_string();
                    if branch == "HEAD" {
                        self.fallback_default_branch(machine_str, repo_dir)
                    } else {
                        branch
                    }
                } else {
                    trimmed
                }
            }
            Err(_) => self.fallback_default_branch(machine_str, repo_dir),
        };

        // 2. Detect PR/MR template
        let pr_template_paths = [
            ".github/pull_request_template.md",
            ".github/PULL_REQUEST_TEMPLATE.md",
            "pull_request_template.md",
            ".gitlab/merge_request_templates/default.md",
            "merge_request_templates/default.md",
        ];
        let mut pr_template = None;
        for path in &pr_template_paths {
            let full_path = format!("{}/{}", repo_dir, path);
            if let Ok(content) = self.exec.read_file(machine_str, &full_path) {
                pr_template = Some(content);
                break;
            }
        }

        // 3. Infer test command
        let mut test_command = None;
        if self
            .exec
            .get_metadata(machine_str, &format!("{}/package.json", repo_dir))
            .is_ok()
        {
            test_command = Some("npm test".to_string());
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/Cargo.toml", repo_dir))
            .is_ok()
        {
            test_command = Some("cargo test".to_string());
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/go.mod", repo_dir))
            .is_ok()
        {
            test_command = Some("go test ./...".to_string());
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/requirements.txt", repo_dir))
            .is_ok()
        {
            test_command = Some("pytest".to_string());
        }

        // 4. Auto-detect project conventions file (for {{project_conventions}} injection).
        // Priority order: AGENTS.md, CLAUDE.md, .cursor/rules/rules.md
        let conventions_candidates = ["AGENTS.md", "CLAUDE.md", ".cursor/rules/rules.md"];
        let mut conventions_file = None;
        for candidate in &conventions_candidates {
            let full_path = format!("{}/{}", repo_dir, candidate);
            if self.exec.get_metadata(machine_str, &full_path).is_ok() {
                conventions_file = Some(full_path);
                break;
            }
        }

        // 5. Infer build command
        let build_command = if self
            .exec
            .get_metadata(machine_str, &format!("{}/package.json", repo_dir))
            .is_ok()
        {
            Some("npm run build".to_string())
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/Cargo.toml", repo_dir))
            .is_ok()
        {
            Some("cargo build".to_string())
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/go.mod", repo_dir))
            .is_ok()
        {
            Some("go build ./...".to_string())
        } else {
            None
        };

        Ok(WorktreeStrategy {
            default_branch,
            branch_prefix: "demeteo/features/".to_string(),
            test_command,
            build_command,
            coverage_command: None,
            conventions_file,
            pr_template,
            harnesses: None,
        })
    }

    fn fallback_default_branch(&self, machine_str: &str, repo_dir: &str) -> String {
        let local_head = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --abbrev-ref HEAD",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .unwrap_or_else(|_| "main".to_string());
        let local_trimmed = local_head.trim();
        if local_trimmed.contains("features/")
            || local_trimmed.contains("subtask")
            || local_trimmed.starts_with("f-")
        {
            "main".to_string()
        } else {
            local_trimmed.to_string()
        }
    }

    /// Check if a repository has uncommitted changes or unpushed commits
    pub fn check_repo_dirty(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String> {
        let machine_str = machine_id.unwrap_or("local");

        // Check if directory exists
        let exists = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --is-inside-work-tree",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .is_ok();

        if !exists {
            return Ok((false, false));
        }

        // 1. Check for uncommitted changes
        let status_output = match self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} status --porcelain",
                paths::shell_escape_posix(repo_dir)
            ),
        ) {
            Ok(out) => out.trim().to_string(),
            Err(e) => return Err(format!("Failed to run git status: {}", e)),
        };
        let has_uncommitted = !status_output.is_empty();

        // 2. Check for unpushed commits
        let unpushed_output = match self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} log --branches --not --remotes --oneline",
                paths::shell_escape_posix(repo_dir)
            ),
        ) {
            Ok(out) => out.trim().to_string(),
            Err(_) => String::new(),
        };
        let has_unpushed = !unpushed_output.is_empty();

        Ok((has_uncommitted, has_unpushed))
    }

    /// Get the current HEAD branch for a repo directory
    pub fn get_head_branch(&self, machine_id: Option<&str>, repo_dir: &str) -> Option<String> {
        let machine_str = machine_id.unwrap_or("local");
        self.exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --abbrev-ref HEAD",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Parse `git worktree list --porcelain` output for a repo directory.
    /// Returns a list of worktrees (excluding the main one) with their branch and lock status.
    pub fn list_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String> {
        let machine_str = machine_id.unwrap_or("local");
        let output = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} worktree list --porcelain",
                paths::shell_escape_posix(repo_dir)
            ),
        )?;

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
    pub fn create_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch_name: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        // Ensure default branch is checked out
        let _ = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} checkout {}",
                paths::shell_escape_posix(repo_dir),
                paths::shell_escape_posix(default_branch)
            ),
        );

        // Try creating and checking out the branch. If it exists, checkout.
        let cmd = format!(
            "git -C {} checkout -b {}",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(branch_name)
        );
        match self.exec.run_command(machine_str, &cmd) {
            Ok(_) => Ok(()),
            Err(_) => {
                let cmd_exists = format!(
                    "git -C {} checkout {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(branch_name)
                );
                self.exec.run_command(machine_str, &cmd_exists).map(|_| ())
            }
        }
    }

    /// Provision a linked worktree for a subtask branched off the main feature branch.
    /// Returns the absolute path to the provisioned worktree.
    pub fn provision_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<String, String> {
        let machine_str = machine_id.unwrap_or("local");
        let wt_dir = format!("{}_wt_{}", repo_dir, subtask_id);
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);

        let _ = self.exec.run_command(
            machine_str,
            &format!("rm -rf {}", paths::shell_escape_posix(&wt_dir)),
        );
        let _ = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} worktree prune",
                paths::shell_escape_posix(repo_dir)
            ),
        );

        let cmd = format!(
            "git -C {} worktree add {} -b {} {}",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(&wt_dir),
            paths::shell_escape_posix(&subtask_branch),
            paths::shell_escape_posix(feature_branch)
        );
        match self.exec.run_command(machine_str, &cmd) {
            Ok(_) => {}
            Err(_) => {
                let fallback_cmd = format!(
                    "git -C {} worktree add {} {}",
                    paths::shell_escape_posix(repo_dir),
                    paths::shell_escape_posix(&wt_dir),
                    paths::shell_escape_posix(&subtask_branch)
                );
                self.exec.run_command(machine_str, &fallback_cmd)?;
            }
        }
        Ok(wt_dir)
    }

    /// Clean up a linked worktree for a subtask, including its branch.
    pub fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let wt_dir = format!("{}_wt_{}", repo_dir, subtask_id);
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);

        let _ = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} worktree remove --force {}",
                paths::shell_escape_posix(repo_dir),
                paths::shell_escape_posix(&wt_dir)
            ),
        );
        let _ = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} worktree prune",
                paths::shell_escape_posix(repo_dir)
            ),
        );
        let _ = self.exec.run_command(
            machine_str,
            &format!("rm -rf {}", paths::shell_escape_posix(&wt_dir)),
        );
        let _ = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} branch -D {}",
                paths::shell_escape_posix(repo_dir),
                paths::shell_escape_posix(&subtask_branch)
            ),
        );
        Ok(())
    }

    /// Check if a subtask is already merged or would conflict, without
    /// touching any working tree. Uses `git fetch` + `git merge-base` +
    /// `git merge-tree` — all pure ref/object operations.
    ///
    /// `repo_dir` is the main clone (used only for its `.git` refs/objects).
    pub fn precheck_merge(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_branch: &str,
    ) -> Result<MergePreCheck, String> {
        let machine_str = machine_id.unwrap_or("local");
        let safe_dir = paths::shell_escape_posix(repo_dir);

        // Fetch latest feature branch from origin into shared refs.
        let _ = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} fetch origin {}",
                safe_dir,
                paths::shell_escape_posix(feature_branch),
            ),
        );

        // Already merged?  subtask is an ancestor of origin/feature_branch.
        if self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} merge-base --is-ancestor {} refs/remotes/origin/{}",
                    safe_dir,
                    paths::shell_escape_posix(subtask_branch),
                    paths::shell_escape_posix(feature_branch),
                ),
            )
            .is_ok()
        {
            return Ok(MergePreCheck::AlreadyMerged);
        }

        // Would conflict?  In-memory merge (no working tree touched).
        match self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} merge-tree --write-tree refs/remotes/origin/{} {}",
                safe_dir,
                paths::shell_escape_posix(feature_branch),
                paths::shell_escape_posix(subtask_branch),
            ),
        ) {
            Ok(_) => Ok(MergePreCheck::CleanMerge),
            Err(_) => Ok(MergePreCheck::WouldConflict),
        }
    }

    /// Merge a subtask branch back into the parent feature branch.
    ///
    /// Operates in the **worktree** (`wt_path`) instead of the main repo
    /// so concurrent pipelines cannot race on a shared checkout.
    pub fn merge_subtask(
        &self,
        machine_id: Option<&str>,
        wt_path: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);
        let safe_wt = paths::shell_escape_posix(wt_path);
        let safe_fb = paths::shell_escape_posix(feature_branch);
        let safe_sb = paths::shell_escape_posix(&subtask_branch);

        // Checkout the feature branch in the worktree, then merge.
        self.exec.run_command(
            machine_str,
            &format!("git -C {} checkout {}", safe_wt, safe_fb),
        )?;

        let cmd = format!(
            "git -C {} merge {} -m \"Merge subtask {}\"",
            safe_wt,
            safe_sb,
            paths::shell_escape_posix(subtask_id),
        );
        self.exec.run_command(machine_str, &cmd)?;
        Ok(())
    }

    /// Fetch the latest state of `default_branch` from `origin` and
    /// hard-reset the local copy of that branch to match. This is the
    /// one-time "snapshot" call used at feature start to make sure the
    /// local default_branch doesn't fall behind after other PRs have
    /// been merged upstream.
    ///
    /// Idempotent and safe to re-invoke: it does not touch any feature
    /// branches, only the local `default_branch` ref.
    pub fn ensure_default_branch_updated(
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
        let _ = self.exec.run_command(
            machine_str,
            &format!("git -C {} fetch origin {}", safe_dir, safe_branch),
        );

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
        self.exec.run_command(
            machine_str,
            &format!("git -C {} checkout {}", safe_dir, safe_branch),
        )?;
        self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} reset --hard {}",
                safe_dir,
                paths::shell_escape_posix(&tracking)
            ),
        )?;
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
    pub fn sync_feature_with_upstream(
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
        let fetch_outcome = self.exec.run_command(
            machine_str,
            &format!("git -C {} fetch origin {}", safe_dir, safe_default),
        );
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
            .map_err(|e| SyncFailure {
                files: Vec::new(),
                raw_error: e,
                worktree_path: None,
            })?;
        let safe_wt = paths::shell_escape_posix(&wt_path);
        let merge_out = self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} merge {} -m \"Sync feature with origin/{}\"",
                safe_wt,
                paths::shell_escape_posix(&tracking),
                default_branch
            ),
        );

        let result = match merge_out {
            Ok(_) => {
                let head_after = self
                    .exec
                    .run_command(machine_str, &format!("git -C {} rev-parse HEAD", safe_wt))
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
                    if let Err(push_err) = self.exec.run_command(machine_str, &push_cmd) {
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
                let files = parse_unmerged_files(&*self.exec, machine_str, &wt_path);
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
            let _ = self.exec.run_command(
                machine_str,
                &format!("git -C {} worktree remove --force {}", safe_dir, safe_wt),
            );
            let _ = self
                .exec
                .run_command(machine_str, &format!("rm -rf {}", safe_wt));
            let _ = self
                .exec
                .run_command(machine_str, &format!("git -C {} worktree prune", safe_dir));
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
    fn provision_sync_worktree(
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
            .unwrap_or_default();
        if current_branch == feature_branch {
            return Ok(repo_dir.to_string());
        }

        let safe_dir = paths::shell_escape_posix(repo_dir);

        // Clean up any stale sync worktrees checked out on this branch
        if let Ok(worktrees) = self.list_worktrees(Some(machine_str), repo_dir) {
            for wt in worktrees {
                if wt.branch.as_deref() == Some(feature_branch) && wt.path.contains("_wt_sync") {
                    let safe_wt_path = paths::shell_escape_posix(&wt.path);
                    let _ = self.exec.run_command(
                        machine_str,
                        &format!(
                            "git -C {} worktree remove --force {}",
                            safe_dir, safe_wt_path
                        ),
                    );
                    let _ = self
                        .exec
                        .run_command(machine_str, &format!("rm -rf {}", safe_wt_path));
                }
            }
            let _ = self
                .exec
                .run_command(machine_str, &format!("git -C {} worktree prune", safe_dir));
        }

        // Use a deterministic path for this feature branch's sync worktree
        let wt_path = format!("{}_wt_sync_{}", repo_dir, feature_branch.replace('/', "_"));
        let safe_wt = paths::shell_escape_posix(&wt_path);

        // Force remove any pre-existing worktree at that path to avoid collisions
        let _ = self.exec.run_command(
            machine_str,
            &format!("git -C {} worktree remove --force {}", safe_dir, safe_wt),
        );
        let _ = self
            .exec
            .run_command(machine_str, &format!("rm -rf {}", safe_wt));
        let _ = self
            .exec
            .run_command(machine_str, &format!("git -C {} worktree prune", safe_dir));

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
            .map_err(|e| {
                format!(
                    "Failed to create sync worktree for '{}': {}",
                    feature_branch, e
                )
            })?;

        Ok(wt_path)
    }
}

/// Result of a successful feature branch sync.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// SHA of the merge commit (empty when there was nothing to merge).
    pub merge_commit_sha: String,
    /// `false` when `origin/<default>` didn't exist or had no new
    /// commits since the last sync.
    pub changed: bool,
}

/// Result of a failed sync — the merge left the working tree in a
/// conflicted state. The caller is expected to spawn a resolution
/// agent or hand the files back to the user.
#[derive(Debug, Clone)]
pub struct SyncFailure {
    pub files: Vec<crate::domain::models::ConflictFile>,
    pub raw_error: String,
    /// Path to the sync worktree where the conflicted state lives.
    /// `None` when the sync was aborted before a worktree was created.
    pub worktree_path: Option<String>,
}

/// Result of a pre-merge check (no working tree touched).
#[derive(Debug, Clone, PartialEq)]
pub enum MergePreCheck {
    /// Subtask is already an ancestor of origin/feature_branch — skip.
    AlreadyMerged,
    /// Merge would be clean — proceed safely.
    CleanMerge,
    /// Merge would produce conflicts — use the resolver cascade.
    WouldConflict,
}

/// Walk `git status --porcelain` and pull out the unmerged paths.
/// Shared by the sync flow and the existing `merge_subtask` conflict
/// path so both produce the same `ConflictFile` shape.
fn parse_unmerged_files(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    repo_dir: &str,
) -> Vec<crate::domain::models::ConflictFile> {
    use crate::domain::models::ConflictFile;
    let raw = match exec.run_command(
        machine_id,
        &format!(
            "git -C {} status --porcelain --untracked-files=no",
            paths::shell_escape_posix(repo_dir)
        ),
    ) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::database::SqliteAdapter;
    use crate::adapters::local::execution::LocalSubprocessAdapter;
    use rusqlite::Connection;
    use std::path::PathBuf;

    #[test]
    fn test_detect_worktree_strategy_local() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_gitops_detect_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        // Run git init and config
        let local_exec = LocalSubprocessAdapter::new();
        let _ = local_exec.run_command(
            "local",
            &format!("git -C \"{}\" init -b main", temp_dir.to_string_lossy()),
        );
        // Create mock files
        local_exec
            .write_file(
                "local",
                &format!("{}/package.json", temp_dir.to_string_lossy()),
                "{}",
            )
            .unwrap();
        local_exec
            .write_file(
                "local",
                &format!(
                    "{}/.github/pull_request_template.md",
                    temp_dir.to_string_lossy()
                ),
                "PR Template Content",
            )
            .unwrap();
        // Commit so HEAD branch is set
        let _ = local_exec.run_command(
            "local",
            &format!(
                "git -C \"{}\" config user.email \"test@demeteo.com\"",
                temp_dir.to_string_lossy()
            ),
        );
        let _ = local_exec.run_command(
            "local",
            &format!(
                "git -C \"{}\" config user.name \"test\"",
                temp_dir.to_string_lossy()
            ),
        );
        let _ = local_exec.run_command(
            "local",
            &format!("git -C \"{}\" add .", temp_dir.to_string_lossy()),
        );
        let _ = local_exec.run_command(
            "local",
            &format!(
                "git -C \"{}\" commit -m \"Initial commit\"",
                temp_dir.to_string_lossy()
            ),
        );

        // Initialize helper
        let conn = Connection::open_in_memory().unwrap();
        let db_adapter =
            Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
        let git_ops = GitOpsHelper::new(db_adapter, Arc::new(local_exec));

        let strategy = git_ops
            .detect_worktree_strategy(None, &temp_dir.to_string_lossy())
            .unwrap();
        assert_eq!(strategy.default_branch, "main");
        assert_eq!(strategy.test_command, Some("npm test".to_string()));
        assert_eq!(
            strategy.pr_template,
            Some("PR Template Content".to_string())
        );

        // Cleanup
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    /// Helper: create a fresh git repo in a temp dir and return (repo_dir, git_ops).
    fn make_repo(suffix: &str) -> (std::path::PathBuf, GitOpsHelper) {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_{}_{}",
            suffix,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let exec = LocalSubprocessAdapter::new();
        let repo = temp_dir.to_string_lossy().to_string();

        let _ = exec.run_command("local", &format!("git -C \"{repo}\" init -b main"));
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{repo}\" config user.email \"ci@demeteo.com\""),
        );
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{repo}\" config user.name \"CI\""),
        );
        exec.write_file("local", &format!("{repo}/README.md"), "# test")
            .unwrap();
        let _ = exec.run_command("local", &format!("git -C \"{repo}\" add ."));
        let _ = exec.run_command("local", &format!("git -C \"{repo}\" commit -m \"init\""));

        let conn = Connection::open_in_memory().unwrap();
        let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
        let helper = GitOpsHelper::new(db, Arc::new(exec));
        (temp_dir, helper)
    }

    #[test]
    fn test_get_head_branch_returns_main() {
        let (dir, helper) = make_repo("head_branch");
        let branch = helper.get_head_branch(None, &dir.to_string_lossy());
        assert_eq!(
            branch,
            Some("main".to_string()),
            "Expected HEAD to be 'main' after `git init -b main`"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_get_head_branch_missing_dir_returns_none() {
        let conn = Connection::open_in_memory().unwrap();
        let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
        let helper = GitOpsHelper::new(db, Arc::new(LocalSubprocessAdapter::new()));
        let result = helper.get_head_branch(None, "/tmp/demeteo_nonexistent_repo_xyz");
        assert!(
            result.is_none(),
            "Expected None for a path that is not a git repo"
        );
    }

    #[test]
    fn test_list_worktrees_only_main_when_no_worktrees_added() {
        let (dir, helper) = make_repo("wt_main_only");
        let worktrees = helper.list_worktrees(None, &dir.to_string_lossy()).unwrap();
        // list_worktrees skips the primary worktree entry, so the result is empty
        assert!(
            worktrees.is_empty(),
            "Expected no additional worktrees beyond the main checkout, got: {:?}",
            worktrees
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn test_list_worktrees_with_one_extra_worktree() {
        let (dir, helper) = make_repo("wt_extra");
        // Canonicalize to handle macOS /tmp → /private/tmp symlink.
        // TempDir may return the symlink path while git worktree list
        // returns the real path, causing an assertion mismatch.
        let repo = std::fs::canonicalize(&dir)
            .unwrap_or_else(|_| dir.as_os_str().to_os_string().into())
            .to_string_lossy()
            .to_string();

        // Add a linked worktree on a new branch
        let wt_dir = format!("{}-wt", repo);
        let exec_tmp = LocalSubprocessAdapter::new();
        let _ = exec_tmp.run_command(
            "local",
            &format!("git -C \"{repo}\" worktree add \"{wt_dir}\" -b feature/my-task"),
        );

        let worktrees = helper.list_worktrees(None, &repo).unwrap();
        assert_eq!(worktrees.len(), 1, "Expected exactly one linked worktree");
        let wt = &worktrees[0];
        assert_eq!(wt.path, wt_dir, "Worktree path should match the added dir");
        assert_eq!(
            wt.branch.as_deref(),
            Some("feature/my-task"),
            "Branch name should be stripped of 'refs/heads/' prefix"
        );
        assert!(!wt.is_locked, "Newly added worktree should not be locked");

        // Cleanup (prune first so git lets us remove the dir)
        let _ = exec_tmp.run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_dir}\""),
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&wt_dir);
    }

    #[test]
    fn test_provision_subtask_worktree_fallback_when_branch_exists() {
        let (dir, helper) = make_repo("wt_fallback");
        let repo = dir.to_string_lossy().to_string();

        // Create the subtask branch manually first so that creating it again via -b fails
        let exec = LocalSubprocessAdapter::new();
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{repo}\" branch main_subtask_sub-1"),
        );

        // Now provision the worktree — it should fall back to checking out the existing branch and succeed
        let wt_path = helper
            .provision_subtask_worktree(None, &repo, "main", "sub-1")
            .unwrap();

        // Verify the worktree path exists
        assert!(std::path::Path::new(&wt_path).exists());

        // Cleanup
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{repo}\" worktree remove --force \"{wt_path}\""),
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&wt_path);
    }

    /// Set up two local repos and wire them together as fake
    /// origin/main. The "remote" is a regular working tree that
    /// we push to via a bare-clone URL; the "local" is a normal
    /// working tree that we sync from. Both start with the same
    /// initial commit. The caller mutates each side to set up the
    /// upstream/feature divergence before calling
    /// `sync_feature_with_upstream`.
    fn make_two_repos(suffix: &str) -> (PathBuf, PathBuf, GitOpsHelper) {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let remote_dir =
            std::env::temp_dir().join(format!("demeteo_test_remote_{}_{}", suffix, stamp));
        let local_dir =
            std::env::temp_dir().join(format!("demeteo_test_local_{}_{}", suffix, stamp));
        std::fs::create_dir_all(&remote_dir).unwrap();
        std::fs::create_dir_all(&local_dir).unwrap();
        let exec = LocalSubprocessAdapter::new();

        // 1. The "remote" is a regular working tree that we push
        //    to. We disable the safety check so we can push to the
        //    currently checked-out branch.
        let remote = remote_dir.to_string_lossy().to_string();
        let _ = exec.run_command("local", &format!("git init -b main \"{remote}\""));
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{remote}\" config user.email \"ci@demeteo.com\""),
        );
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{remote}\" config user.name \"CI\""),
        );
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{remote}\" config receive.denyCurrentBranch ignore"),
        );
        exec.write_file("local", &format!("{remote}/README.md"), "init")
            .unwrap();
        let _ = exec.run_command("local", &format!("git -C \"{remote}\" add ."));
        let _ = exec.run_command("local", &format!("git -C \"{remote}\" commit -m init"));

        // 2. The "local" is a clone of the remote so it shares the
        //    initial commit and has `origin` already wired up.
        let local = local_dir.to_string_lossy().to_string();
        let _ = exec.run_command("local", &format!("git clone \"{remote}\" \"{local}\""));
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" config user.email \"ci@demeteo.com\""),
        );
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" config user.name \"CI\""),
        );

        let conn = Connection::open_in_memory().unwrap();
        let db = Arc::new(SqliteAdapter::new(conn).unwrap()) as Arc<dyn AppSettingsRepository>;
        let helper = GitOpsHelper::new(db, Arc::new(exec));
        (local_dir, remote_dir, helper)
    }

    /// The exact bug the user hit: a feature branch is "2 commits
    /// behind" main with overlapping changes. The sync must
    /// surface the conflict list, not silently return "no new
    /// commits upstream".
    #[test]
    fn test_sync_feature_with_upstream_detects_conflicts() {
        let (local_dir, remote_dir, helper) = make_two_repos("sync_conflict");
        let local = local_dir.to_string_lossy().to_string();
        let remote = remote_dir.to_string_lossy().to_string();
        let exec = LocalSubprocessAdapter::new();

        // 1. Create a feature branch with a change to README.md.
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        );
        exec.write_file("local", &format!("{local}/README.md"), "feature change")
            .unwrap();
        let _ = exec.run_command("local", &format!("git -C \"{local}\" commit -am feature"));

        // 2. Advance upstream main (the "remote" working tree)
        //    with an *overlapping* change to the same line. The
        //    user's bug was that this never surfaced as a conflict
        //    when the local feature branch synced.
        exec.write_file("local", &format!("{remote}/README.md"), "main change")
            .unwrap();
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{remote}\" commit -am main-advance"),
        );

        // 3. Sync the feature branch with origin/main. We expect a
        //    conflict (because the README.md was edited on both
        //    sides), not a silent "no new commits upstream".
        let outcome = helper.sync_feature_with_upstream(None, &local, "feature/f-1", "main");

        match outcome {
            Ok(_) => panic!(
                "Expected a conflict, but sync returned Ok. The user's bug: \
                 the merge should have failed because README.md was edited on \
                 both sides."
            ),
            Err(failure) => {
                assert!(
                    !failure.files.is_empty(),
                    "Sync reported failure but no conflict files were captured. \
                     raw_error: {}",
                    failure.raw_error
                );
                assert!(
                    failure.files.iter().any(|f| f.path == "README.md"),
                    "README.md should be in the conflict list, got: {:?}",
                    failure.files
                );
            }
        }

        let _ = std::fs::remove_dir_all(&local_dir);
        let _ = std::fs::remove_dir_all(&remote_dir);
    }

    /// When the feature branch already includes all of upstream
    /// main, the sync is a true no-op and must say so
    /// (`changed: false`) — not invent a merge commit.
    #[test]
    fn test_sync_feature_with_upstream_noop_when_already_in_sync() {
        let (local_dir, remote_dir, helper) = make_two_repos("sync_noop");
        let local = local_dir.to_string_lossy().to_string();
        let exec = LocalSubprocessAdapter::new();

        // Feature branch on top of the same commit as main.
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        );

        let outcome = helper
            .sync_feature_with_upstream(None, &local, "feature/f-1", "main")
            .expect("Sync should succeed when there is nothing to merge");

        assert!(
            !outcome.changed,
            "Sync must report `changed: false` when the feature branch already \
             matches origin/main; got: changed={}",
            outcome.changed
        );

        let _ = std::fs::remove_dir_all(&local_dir);
        let _ = std::fs::remove_dir_all(&remote_dir);
    }

    /// When origin is unreachable the sync must surface a real
    /// error so the user knows the merge wasn't actually attempted.
    /// (The old code silently swallowed fetch failures.)
    #[test]
    fn test_sync_feature_with_upstream_reports_fetch_failure() {
        let (local_dir, remote_dir, helper) = make_two_repos("sync_fetch_fail");
        let local = local_dir.to_string_lossy().to_string();
        let exec = LocalSubprocessAdapter::new();

        // Create a feature branch and break the remote so the fetch
        // will fail (pointing at a nonexistent path).
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-1"),
        );
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" remote set-url origin /nonexistent/path"),
        );

        let outcome = helper.sync_feature_with_upstream(None, &local, "feature/f-1", "main");
        match outcome {
            Ok(o) => panic!(
                "Sync must NOT return Ok when the fetch fails. Got: {:?}. \
                 The user's bug was that fetch errors were silently swallowed \
                 and the caller saw a misleading 'no new commits upstream'.",
                o
            ),
            Err(failure) => {
                assert!(
                    failure.raw_error.to_lowercase().contains("fetch")
                        || failure.raw_error.to_lowercase().contains("origin")
                        || failure.raw_error.to_lowercase().contains("remote"),
                    "Error message should mention the fetch/remote failure, got: {}",
                    failure.raw_error
                );
            }
        }

        let _ = std::fs::remove_dir_all(&local_dir);
        let _ = std::fs::remove_dir_all(&remote_dir);
    }

    /// The user hit this bug: after `sync_feature_with_upstream`
    /// produced a conflict, the resolver (which used a fresh
    /// worktree) found a clean working tree, the agent had nothing
    /// to fix, and the commit failed with "nothing to commit".
    /// This test pins the property: the conflict lives in the
    /// main repo's index and working tree, and that is exactly
    /// where the agent must run. A fresh worktree is NOT a
    /// substitute.
    #[test]
    fn test_resolver_must_run_in_main_repo_not_worktree() {
        let (local_dir, remote_dir, helper) = make_two_repos("wt_not_inherit");
        let local = local_dir.to_string_lossy().to_string();
        let remote = remote_dir.to_string_lossy().to_string();
        let exec = LocalSubprocessAdapter::new();

        // 1. Create a feature branch with an overlapping change.
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" checkout -b feature/f-resolver"),
        );
        exec.write_file("local", &format!("{local}/README.md"), "feature change")
            .unwrap();
        let _ = exec.run_command("local", &format!("git -C \"{local}\" commit -am feature"));

        // 2. Advance upstream with an overlapping change.
        exec.write_file("local", &format!("{remote}/README.md"), "main change")
            .unwrap();
        let _ = exec.run_command("local", &format!("git -C \"{remote}\" commit -am advance"));

        // 3. Sync in the main repo — leaves it conflicted.
        let _ = helper.sync_feature_with_upstream(None, &local, "feature/f-resolver", "main");

        // 4. Critical assertion: the main repo's working tree DOES
        //    contain the conflict. This is what the resolver must
        //    operate on.
        let main_status = exec
            .run_command(
                "local",
                &format!("git -C \"{local}\" status --porcelain --untracked-files=no"),
            )
            .unwrap();
        assert!(
            main_status.contains("README.md"),
            "Main repo should have README.md in unmerged state; got: {}",
            main_status
        );

        // 5. Critical assertion: a fresh worktree off the same
        //    branch does NOT carry the conflict state. The naive
        //    "provision a worktree and spawn the agent there"
        //    pattern would have the agent see a clean tree and
        //    commit nothing. This is the bug the user hit.
        let wt_path = helper
            .provision_subtask_worktree(None, &local, "feature/f-resolver", "sub-resolver")
            .unwrap();
        let wt = wt_path.clone();
        let wt_status = exec
            .run_command(
                "local",
                &format!("git -C \"{wt}\" status --porcelain --untracked-files=no"),
            )
            .unwrap();
        assert!(
            wt_status.trim().is_empty(),
            "A fresh worktree MUST start clean (the conflict state lives in \
             the main repo's index, not in any worktree's index). If this \
             assertion fails the resolver is in the wrong place. Got: {}",
            wt_status
        );

        // Cleanup
        let _ = exec.run_command(
            "local",
            &format!("git -C \"{local}\" worktree remove --force \"{wt}\""),
        );
        let _ = std::fs::remove_dir_all(&local_dir);
        let _ = std::fs::remove_dir_all(&remote_dir);
    }
}
