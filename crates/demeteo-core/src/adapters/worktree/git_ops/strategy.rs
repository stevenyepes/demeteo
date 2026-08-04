use super::GitOpsHelper;
use crate::domain::ecosystem::{self, MarkerSite, ECOSYSTEMS, JS_LOCKFILES, MAX_SCANNED_SUBDIRS};
use crate::domain::models::{ScriptVariants, WorktreeStrategy};
use crate::ports::execution::ProgramRequest;
use std::collections::HashMap;

impl GitOpsHelper {
    /// Run git analysis and propose strategy settings
    pub async fn detect_worktree_strategy(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<WorktreeStrategy, String> {
        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);

        // 1. Detect Default Branch name
        // Try origin/HEAD first. Fallback to local HEAD, but reject feature/subtask branch names.
        let default_branch = match self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--abbrev-ref", "origin/HEAD"]),
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

        // 3. Find every ecosystem in the repository, then let
        // `domain::ecosystem` decide what to emit for it.
        //
        // A polyglot repo (a Tauri app is package.json *and* Cargo.toml; a Go
        // service with a JS frontend is go.mod *and* package.json) needs
        // *every* ecosystem's suite to run. First-match-wins picked one command
        // and silently dropped the rest, so the harness could run only
        // `cargo test` for a TypeScript-only change — a gate that passes while
        // the change's real suite never executed.
        //
        // Running all of them used to mean chaining them into one string with a
        // hand-rolled `rc` accumulator, because there was nowhere to put more
        // than one harness. There is now: `harnesses` is plural and
        // gate-selectable (HB5), so each ecosystem gets its own named gate and a
        // failure says *which* one went red — the attribution HB2c's per-gate
        // subtraction reads. The accumulator is deleted rather than fixed.
        //
        // A user can still override the lot with a single explicit command
        // (e.g. a repo's own `npm run checks` aggregate) in settings.
        let sites = self.find_marker_sites(machine_str, repo_dir).await;
        let detected = ecosystem::compose_commands(&sites);

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
            test_command: detected.test_command.map(posix_script),
            build_command: detected.build_command.map(posix_script),
            coverage_command: None,
            conventions_file,
            pr_template,
            harnesses: Some(
                detected
                    .harnesses
                    .into_iter()
                    .map(|(name, command)| (name, posix_script(command)))
                    .collect::<HashMap<_, _>>(),
            )
            .filter(|h| !h.is_empty()),
            validation_gates: Some(detected.validation_gates).filter(|g| !g.is_empty()),
            prepare_command: detected.prepare_command.map(posix_script),
            extra_writable_paths: Vec::new(),
        })
    }

    /// Locate every ecosystem marker in the repository, at a **bounded** depth,
    /// and gather the evidence `domain::ecosystem` needs to resolve each one.
    ///
    /// # Why below the root at all
    ///
    /// Detection used to stat `{repo_dir}/{marker}` and nothing deeper. A Tauri
    /// app keeps its `Cargo.toml` in `src-tauri/`, so it matched `package.json`
    /// alone — the entire Rust half of the project was invisible to detection
    /// and the generated command silently covered half the repo.
    ///
    /// # Why not deeper
    ///
    /// An unbounded walk on a monorepo is its own bug, so this goes exactly one
    /// level down and no further, skipping the directories that are full of
    /// other people's manifests ([`ecosystem::SKIPPED_DIRS`], every dot-dir) and
    /// stopping after [`MAX_SCANNED_SUBDIRS`]. A marker at depth two belongs to
    /// a layout a human should be describing in settings.
    ///
    /// # Why `list_dir` and not `get_metadata`
    ///
    /// One listing answers every question about a directory at once — which
    /// markers are in it, which lockfiles sit beside them, and which
    /// subdirectories are worth visiting. Stat-ing each marker in each candidate
    /// directory would be four round trips per directory instead of one, which
    /// over SSH is the difference between a detection and a wait. If the root
    /// listing fails there is no fallback worth writing: a repository directory
    /// that cannot be listed cannot be stat-ed into either.
    ///
    /// Paths are joined with `/` rather than `PathBuf::join` because they are
    /// addresses on the *target* machine, which is Linux for every remote
    /// project regardless of what the desktop runs — see
    /// [`ecosystem::in_dir`]'s note.
    async fn find_marker_sites(&self, machine_str: &str, repo_dir: &str) -> Vec<MarkerSite> {
        let Ok(root) = self.exec.list_dir(machine_str, repo_dir).await else {
            return Vec::new();
        };

        let mut sites = self.sites_in(machine_str, repo_dir, "", &root).await;

        let subdirs: Vec<String> = root
            .iter()
            .filter(|e| e.is_dir && ecosystem::is_scannable_subdir(&e.name))
            .map(|e| e.name.clone())
            .take(MAX_SCANNED_SUBDIRS)
            .collect();
        for dir in subdirs {
            let path = format!("{}/{}", repo_dir, dir);
            if let Ok(entries) = self.exec.list_dir(machine_str, &path).await {
                sites.extend(self.sites_in(machine_str, &path, &dir, &entries).await);
            }
        }
        sites
    }

    /// Turn one directory listing into the [`MarkerSite`]s it contains.
    ///
    /// `rel` is the directory's repo-relative path (empty for the root), which
    /// is what the emitted commands are wrapped to run in.
    async fn sites_in(
        &self,
        machine_str: &str,
        abs_dir: &str,
        rel: &str,
        entries: &[crate::ports::execution::SftpEntry],
    ) -> Vec<MarkerSite> {
        let present = |name: &str| entries.iter().any(|e| !e.is_dir && e.name == name);
        let mut sites = Vec::new();
        for recipe in ECOSYSTEMS {
            if !present(recipe.marker) {
                continue;
            }
            // Only the JS recipe reads anything beyond the marker's existence:
            // its commands live in `scripts`, and which manager installs them is
            // in the lockfile. The others are fixed by the toolchain.
            let (manifest, lockfiles) = if recipe.id == "js" {
                (
                    self.exec
                        .read_file(machine_str, &format!("{}/{}", abs_dir, recipe.marker))
                        .await
                        .ok(),
                    JS_LOCKFILES
                        .iter()
                        .filter(|l| present(l))
                        .map(|l| (*l).to_string())
                        .collect(),
                )
            } else {
                (None, Vec::new())
            };
            sites.push(MarkerSite {
                marker: recipe.marker.to_string(),
                dir: rel.to_string(),
                manifest,
                lockfiles,
            });
        }
        sites
    }

    async fn fallback_default_branch(&self, machine_str: &str, repo_dir: &str) -> String {
        let local_head = self
            .exec
            .run_program(
                machine_str,
                git_request(repo_dir, ["rev-parse", "--abbrev-ref", "HEAD"]),
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

fn posix_script(posix: String) -> ScriptVariants {
    ScriptVariants {
        posix: Some(posix),
        powershell: None,
    }
}

fn git_request<const N: usize>(repo_dir: &str, args: [&str; N]) -> ProgramRequest {
    ProgramRequest {
        executable: "git".to_string(),
        args: [
            vec!["-C".to_string(), repo_dir.to_string()],
            args.into_iter().map(str::to_string).collect(),
        ]
        .concat(),
        ..ProgramRequest::default()
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/strategy.rs"]
mod tests;
