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

        // 3. Detect ecosystems once, then derive test + build commands from the
        // same set. A polyglot repo (a Tauri app is package.json *and*
        // Cargo.toml; a Go service with a JS frontend is go.mod *and*
        // package.json) needs *every* ecosystem's suite to run. The old
        // first-match-wins picked a single command and silently dropped the
        // rest, so the verifier's harness could run e.g. only `cargo test` for
        // a TypeScript-only change — a gate that passes while the change's real
        // suite (`tsc`/vitest) never executed, reporting a red build as green
        // and looping the implement retry forever. Collecting the ecosystem set
        // in one pass keeps the test and build command lists from drifting (the
        // build command had the same first-match-wins bug), and stats each
        // marker file only once. A user can still override with a single
        // explicit command (e.g. a repo's own `npm run checks` aggregate) in
        // settings.
        struct Ecosystem {
            marker: &'static str,
            test: &'static str,
            build: Option<&'static str>,
        }
        const ECOSYSTEMS: &[Ecosystem] = &[
            Ecosystem {
                marker: "package.json",
                test: "npm test",
                build: Some("npm run build"),
            },
            Ecosystem {
                marker: "Cargo.toml",
                test: "cargo test",
                build: Some("cargo build"),
            },
            Ecosystem {
                marker: "go.mod",
                test: "go test ./...",
                build: Some("go build ./..."),
            },
            Ecosystem {
                marker: "requirements.txt",
                test: "pytest",
                build: None,
            },
        ];

        let mut test_cmds: Vec<&str> = Vec::new();
        let mut build_cmds: Vec<&str> = Vec::new();
        for eco in ECOSYSTEMS {
            if self
                .exec
                .get_metadata(machine_str, &format!("{}/{}", repo_dir, eco.marker))
                .await
                .is_ok()
            {
                test_cmds.push(eco.test);
                if let Some(b) = eco.build {
                    build_cmds.push(b);
                }
            }
        }

        // Chain all detected suites so every one runs each verify and the
        // harness fails if ANY suite fails (not just the first). `&&` would let
        // an unrelated red in an earlier suite mask or block a change whose real
        // gate runs later; the `rc` accumulator runs every suite and preserves a
        // non-zero exit. Runs under the login shell (`run_command_with`), so
        // `set +e`/`$?`/`exit` behave. A single-ecosystem repo (the common case)
        // keeps the bare command so it reads cleanly in settings.
        let run_all = |cmds: &[&str]| -> Option<String> {
            match cmds {
                [] => None,
                [only] => Some((*only).to_string()),
                _ => {
                    let body = cmds
                        .iter()
                        .map(|c| format!("{c}; rc=$((rc||$?))"))
                        .collect::<Vec<_>>()
                        .join("; ");
                    Some(format!("set +e; rc=0; {body}; exit $rc"))
                }
            }
        };
        let test_command = run_all(&test_cmds);
        let build_command = run_all(&build_cmds);

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

        Ok(WorktreeStrategy {
            default_branch,
            branch_prefix: "demeteo/features/".to_string(),
            test_command,
            build_command,
            coverage_command: None,
            conventions_file,
            pr_template,
            harnesses: None,
            prepare_command: None,
            extra_writable_paths: Vec::new(),
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

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/strategy.rs"]
mod tests;
