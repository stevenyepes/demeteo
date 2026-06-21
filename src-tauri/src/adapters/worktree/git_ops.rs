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
        // Immediately after cloning, checking HEAD is the most reliable way to get the default branch
        let default_branch = match self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} rev-parse --abbrev-ref HEAD",
                paths::shell_escape_posix(repo_dir)
            ),
        ) {
            Ok(out) => out.trim().to_string(),
            Err(_) => "main".to_string(), // Fallback
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
        })
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

    /// Merge a subtask branch back into the parent feature branch.
    pub fn merge_subtask(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        subtask_id: &str,
    ) -> Result<(), String> {
        let machine_str = machine_id.unwrap_or("local");
        let subtask_branch = format!("{}_subtask_{}", feature_branch, subtask_id);

        self.exec.run_command(
            machine_str,
            &format!(
                "git -C {} checkout {}",
                paths::shell_escape_posix(repo_dir),
                paths::shell_escape_posix(feature_branch)
            ),
        )?;

        let cmd = format!(
            "git -C {} merge {} -m \"Merge subtask {}\"",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(&subtask_branch),
            paths::shell_escape_posix(subtask_id)
        );
        self.exec.run_command(machine_str, &cmd)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::database::SqliteAdapter;
    use crate::adapters::local::execution::LocalSubprocessAdapter;
    use rusqlite::Connection;

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
}
