use super::GitOpsHelper;
use crate::domain::models::WorktreeStrategy;
use crate::paths;

impl GitOpsHelper {
    /// Run git analysis and propose strategy settings
    pub async fn detect_worktree_strategy(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<WorktreeStrategy, String> {
        let machine_str = machine_id.unwrap_or("local");

        // 1. Detect Default Branch name
        // Try origin/HEAD first. Fallback to local HEAD, but reject feature/subtask branch names.
        let default_branch = match self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --abbrev-ref origin/HEAD",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await
        {
            Ok(out) => {
                let trimmed = out.trim().to_string();
                if let Some(stripped) = trimmed.strip_prefix("origin/") {
                    let branch = stripped.to_string();
                    if branch == "HEAD" {
                        self.fallback_default_branch(machine_str, repo_dir).await
                    } else {
                        branch
                    }
                } else {
                    trimmed
                }
            }
            Err(_) => self.fallback_default_branch(machine_str, repo_dir).await,
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
            if let Ok(content) = self.exec.read_file(machine_str, &full_path).await {
                pr_template = Some(content);
                break;
            }
        }

        // 3. Infer test command
        let mut test_command = None;
        if self
            .exec
            .get_metadata(machine_str, &format!("{}/package.json", repo_dir))
            .await
            .is_ok()
        {
            test_command = Some("npm test".to_string());
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/Cargo.toml", repo_dir))
            .await
            .is_ok()
        {
            test_command = Some("cargo test".to_string());
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/go.mod", repo_dir))
            .await
            .is_ok()
        {
            test_command = Some("go test ./...".to_string());
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/requirements.txt", repo_dir))
            .await
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
            if self
                .exec
                .get_metadata(machine_str, &full_path)
                .await
                .is_ok()
            {
                conventions_file = Some(full_path);
                break;
            }
        }

        // 5. Infer build command
        let build_command = if self
            .exec
            .get_metadata(machine_str, &format!("{}/package.json", repo_dir))
            .await
            .is_ok()
        {
            Some("npm run build".to_string())
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/Cargo.toml", repo_dir))
            .await
            .is_ok()
        {
            Some("cargo build".to_string())
        } else if self
            .exec
            .get_metadata(machine_str, &format!("{}/go.mod", repo_dir))
            .await
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

    async fn fallback_default_branch(&self, machine_str: &str, repo_dir: &str) -> String {
        let local_head = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --abbrev-ref HEAD",
                    paths::shell_escape_posix(repo_dir)
                ),
            )
            .await
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
}
