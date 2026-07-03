use crate::domain::artifact::{Artifact, ArtifactCapture, ArtifactDecl, ArtifactSource};
use crate::paths;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::execution::ExecutionPort;
use std::sync::Arc;

/// Resolve `declarations` against the `ArtifactProduced` events emitted
/// by the agent during a step turn. Writes matching artifacts through
/// the store and returns the list of references (paths for the FS
/// adapter) to persist in `StepExecution.artifact_paths`.
///
/// Artifacts that cannot be matched are silently skipped with a
/// `tracing::warn!` — the step executor will still mark the step as
/// completed successfully; missing artifacts are a prompt-engineering
/// concern, not a runtime failure.
pub(crate) fn resolve_declared_artifacts(
    declarations: &[ArtifactDecl],
    produced: &[Artifact],
    store: &Arc<dyn ArtifactStore>,
    feature_id: &str,
    step_id: &str,
) -> Vec<String> {
    let mut refs = Vec::new();

    for decl in declarations {
        let matched: Option<&Artifact> = match &decl.capture {
            ArtifactCapture::ByName { name } => produced
                .iter()
                .find(|a| a.name == *name || strip_extension(&a.name).is_some_and(|s| s == *name)),
            ArtifactCapture::LastWriteTo { path } => produced
                .iter()
                .rfind(|a| matches!(&a.source, ArtifactSource::ToolWrite { path: p } if p == path)),
            ArtifactCapture::AllWrites => {
                // Collect all tool-write artifacts. We still produce the
                // named artifacts below; the `AllWrites` catch-all emits
                // one artifact per unique path.
                continue; // handled separately below
            }
            ArtifactCapture::ChangedFiles { .. } => {
                // ChangedFiles artifacts are detected directly via git diff
                // in agent.rs and added to produced_artifacts there. They
                // are named by their file basenames, so they can be matched
                // here by name if needed. We just continue — they are already
                // in the produced list.
                continue;
            }
            ArtifactCapture::Diff { .. } => {
                // Diff artifacts are derived at materialisation time by
                // `GitOpsHelper`. No agent event matches them. The
                // orchestrator should synthesise them at TurnComplete
                // when `GitOpsHelper` methods are available (next step).
                eprintln!(
                    "[artifacts] step={} decl={}: Diff declaration skipped — GitOpsHelper not yet wired",
                    step_id, decl.name,
                );
                continue;
            }
            ArtifactCapture::Worktree { .. } => {
                // Worktree-ref artifacts are synthesised by the executor
                // from branch/machine state. No agent event matches them.
                eprintln!(
                    "[artifacts] step={} decl={}: Worktree declaration skipped — GitOpsHelper not yet wired",
                    step_id, decl.name,
                );
                continue;
            }
        };

        if let Some(artifact) = matched {
            match store.put(feature_id, step_id, artifact) {
                Ok(reference) => refs.push(reference),
                Err(e) => {
                    eprintln!(
                        "[artifacts] step={} decl={}: Failed to store artifact: {}",
                        step_id, decl.name, e,
                    );
                }
            }
        } else {
            eprintln!(
                "[artifacts] step={} decl={}: No matching ArtifactProduced event",
                step_id, decl.name,
            );
        }
    }

    // Handle `AllWrites` catch-all: collect every unique ToolWrite path.
    let has_all_writes = declarations
        .iter()
        .any(|d| matches!(d.capture, ArtifactCapture::AllWrites));
    if has_all_writes {
        let mut seen_paths = std::collections::HashSet::new();
        for artifact in produced {
            if let ArtifactSource::ToolWrite { path } = &artifact.source {
                if seen_paths.insert(path.clone()) {
                    match store.put(feature_id, step_id, artifact) {
                        Ok(reference) => refs.push(reference),
                        Err(e) => {
                            eprintln!(
                            "[artifacts] step={} path={}: Failed to store AllWrites artifact: {}",
                            step_id, path, e,
                        );
                        }
                    }
                }
            }
        }
    }

    refs
}

/// Read the post-write content of `rel_path` (relative to the
/// worktree root) and return it as a string. Skips binary files
/// (those containing a NUL byte in the first 8 KiB) and returns
/// `None` if the file is missing or unreadable.
///
/// This is the "snapshot the agent's working tree" primitive that
/// the step executor calls after the agent turn ends. It is
/// deliberately simple: read the file, drop binaries, and return
/// the body. The orchestrator stores the body as the artifact
/// content and the `rel_path` as the on-disk name suffix in the
/// `FsArtifactStore`.
pub async fn read_worktree_file(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    rel_path: &str,
) -> Option<String> {
    let abs = format!("{}/{}", worktree_root.trim_end_matches('/'), rel_path);
    let content = exec.read_file(machine_id, &abs).await.ok()?;
    if is_likely_binary(&content) {
        return None;
    }
    Some(content)
}

/// Compute the unified diff of the worktree's working tree (and
/// index) against `base_ref`. Returns the diff body as a string,
/// or an empty string if there are no changes or `base_ref` cannot
/// be resolved.
///
/// `base_ref` is whatever `git rev-parse` accepts: a branch name,
/// a SHA, `HEAD`, `HEAD~1`, the worktree's merge-base against the
/// default branch, etc. The diff includes both staged and
/// unstaged changes.
pub async fn compute_git_diff(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    base_ref: &str,
) -> String {
    let cmd = format!(
        "git -C {} diff {}",
        paths::shell_escape_posix(worktree_root),
        paths::shell_escape_posix(base_ref),
    );
    exec.run_command(machine_id, &cmd).await.unwrap_or_default()
}

/// Stage every change in the worktree and commit it with `message`.
/// Used by the parallel step to make `merge_subtask` meaningful
/// (the agent only writes files; the orchestrator has to commit
/// them so the merge has a non-empty tip to bring across).
///
/// Pre-condition: a `user.email` and `user.name` are configured for
/// the worktree's git repo. The orchestrator sets these on
/// bootstrap for the project repo; if they're missing the commit
/// fails with a clear error and the caller treats the step as
/// failed.
///
/// `artifact_subdir` and `commit_artifacts` (migration V12) let the
/// caller keep agent reports (`research-report.md`,
/// `critic-review.md`, …) out of the feature branch by default.
/// When `commit_artifacts` is false the orchestrator runs
/// `git add -A -- ':!<artifact_subdir>'` so the reports stay in
/// the worktree as untracked files instead of being committed.
/// Their content is still captured by `process_agent_artifacts` into
/// the `FsArtifactStore`, so the UI keeps working and no data is
/// lost — the PR just stays clean. When `commit_artifacts` is true
/// the call falls back to a plain `git add -A` (legacy behaviour).
///
/// `non_artifact_writes` is the list of repo-relative paths the
/// agent actually wrote to during this step that are NOT under
/// `artifact_subdir` (i.e. the paths the user actually asked the
/// agent to create or modify). It is used by the guard log below
/// to detect the historical "agent put the doc body in
/// `artifacts/s-…​.md` instead of the real path" failure mode: if
/// these paths exist in the worktree but the stage is empty (or
/// only contains paths under `artifact_subdir`), we emit a
/// `tracing::warn!` so the regression is observable instead of
/// silently producing an empty commit. Pass `&[]` for parallel
/// steps where this signal isn't tracked — the warn still fires
/// for an empty stage, which is the cheaper half of the check.
///
/// Returns the new commit SHA on success, or an error string on
/// failure.
pub async fn commit_worktree_changes(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    message: &str,
    artifact_subdir: &str,
    commit_artifacts: bool,
    non_artifact_writes: &[String],
) -> Result<String, String> {
    // Build the `git add` invocation. When the user has opted out
    // of committing artifacts, use a pathspec exclusion to keep
    // them out of the index. The subdir is repo-relative, so we
    // can drop a leading `./` and trim trailing slashes for a
    // cleaner pathspec.
    let trimmed = artifact_subdir.trim().trim_start_matches("./");
    let trimmed = trimmed.trim_end_matches('/');
    let mut exclusions = String::new();
    if !commit_artifacts && !trimmed.is_empty() {
        exclusions.push_str(&format!(" ':!{trimmed}'"));
    }
    // Exclude any of `paths::DEPENDENCY_CACHE_DIRS` that are actually
    // symlinks in *this* worktree — i.e. the ones `provision_subtask_worktree`
    // linked in from the primary checkout. `git add -A` already skips
    // genuinely gitignored paths, but a symlink standing in for one of
    // these isn't recognized against a trailing-slash `.gitignore` pattern
    // (see `paths::DEPENDENCY_CACHE_DIRS`), so without this exclusion the
    // symlink itself — an absolute host path — would be staged and
    // committed onto the feature branch. Checking `-L` (rather than
    // excluding the names unconditionally) means a project that
    // legitimately tracks a directory sharing one of these names (e.g.
    // Go's vendored `vendor/`) is unaffected — we only ever skip our own
    // symlinks, never a real tracked directory.
    let dirs = crate::paths::DEPENDENCY_CACHE_DIRS.join(" ");
    // `; true` at the end matters: the loop's exit status would otherwise
    // be that of the last `[ -L "$d" ]` test, which is false (non-zero)
    // whenever the final candidate isn't a symlink — `run_command` treats
    // any non-zero exit as `Err` and the whole exclusion list would be
    // silently dropped.
    let symlink_check = format!(
        "cd {} && for d in {}; do [ -L \"$d\" ] && echo \"$d\"; done; true",
        paths::shell_escape_posix(worktree_root),
        dirs,
    );
    if let Ok(out) = exec.run_command(machine_id, &symlink_check).await {
        for name in out.lines().map(str::trim).filter(|s| !s.is_empty()) {
            exclusions.push_str(&format!(" ':!{name}'"));
        }
    }
    let add_paths = if exclusions.is_empty() {
        String::new()
    } else {
        format!(" --{exclusions}")
    };
    let add_cmd = format!(
        "git -C {} add -A{add_paths}",
        paths::shell_escape_posix(worktree_root),
    );
    exec.run_command(machine_id, &add_cmd)
        .await
        .map_err(|e| format!("git add failed: {}", e))?;

    // Guard log: surface the "agent's deliverable landed in
    // `artifacts/` instead of at the real path" regression mode
    // instead of silently producing an empty commit. The two cases
    // we flag are:
    //   (a) the stage is empty even though `non_artifact_writes`
    //       lists paths the agent touched — the agent's writes
    //       either vanished (e.g. permission-scope rejection) or
    //       all went to excluded paths;
    //   (b) the stage only contains paths under `artifact_subdir`
    //       while `non_artifact_writes` lists paths outside it —
    //       the agent emitted the real doc body into its summary
    //       report (`artifacts/s-draft.md`, …) instead of the real
    //       path, so the PR ends up carrying only the report and
    //       the actual deliverable is stranded as an untracked file.
    let diff_cached_cmd = format!(
        "git -C {} diff --cached --name-only",
        paths::shell_escape_posix(worktree_root),
    );
    if let Ok(staged_raw) = exec.run_command(machine_id, &diff_cached_cmd).await {
        let staged: Vec<&str> = staged_raw
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if staged.is_empty() && !non_artifact_writes.is_empty() {
            tracing::warn!(
                worktree = %worktree_root,
                message = %message,
                expected_non_artifact_writes = ?non_artifact_writes,
                "commit_worktree_changes: stage is empty but the agent reported writes outside `{}` — the agent's deliverable did not reach the index",
                artifact_subdir,
            );
        } else if !non_artifact_writes.is_empty() && !staged.is_empty() {
            let trimmed_subdir = artifact_subdir
                .trim()
                .trim_start_matches("./")
                .trim_end_matches('/');
            let stage_has_non_artifact = staged.iter().any(|p| !is_under_prefix(p, trimmed_subdir));
            if !stage_has_non_artifact {
                tracing::warn!(
                    worktree = %worktree_root,
                    message = %message,
                    expected_non_artifact_writes = ?non_artifact_writes,
                    staged = ?staged,
                    "commit_worktree_changes: stage only contains paths under `{}` while the agent reported writes outside it — the agent's doc body likely landed in the summary report instead of the real path",
                    artifact_subdir,
                );
            }
        }
    }

    let commit_cmd = format!(
        "git -C {} -c user.email=demeteo@local -c user.name=demeteo commit -m {} --allow-empty",
        paths::shell_escape_posix(worktree_root),
        paths::shell_escape_posix(message),
    );
    let out = exec
        .run_command(machine_id, &commit_cmd)
        .await
        .map_err(|e| format!("git commit failed: {}", e))?;

    let sha_cmd = format!(
        "git -C {} rev-parse HEAD",
        paths::shell_escape_posix(worktree_root),
    );
    exec.run_command(machine_id, &sha_cmd)
        .await
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("git rev-parse after commit failed: {}", e))
        .inspect(|_sha| {
            if !out.is_empty() {
                eprintln!("[commit_worktree_changes] {}", out.trim());
            }
        })
}

/// True when `path` sits at or under the directory `prefix` (a
/// directory path with no trailing slash, e.g. `"artifacts"`).
/// Matches both the directory itself (`"artifacts"`) and any file
/// inside it (`"artifacts/s-draft.md"`).
pub(crate) fn is_under_prefix(path: &str, prefix: &str) -> bool {
    if prefix.is_empty() {
        return false;
    }
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn is_likely_binary(content: &str) -> bool {
    if content.contains('\0') {
        return true;
    }
    let head = &content[..content.len().min(8192)];
    if head.len() > 256 && !head.contains('\n') {
        return true;
    }
    false
}

fn strip_extension(name: &str) -> Option<String> {
    std::path::Path::new(name)
        .file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/declared.rs"]
mod tests;
