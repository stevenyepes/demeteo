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
/// Returns the new commit SHA on success, or an error string on
/// failure.
pub async fn commit_worktree_changes(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    message: &str,
) -> Result<String, String> {
    let add_cmd = format!("git -C {} add -A", paths::shell_escape_posix(worktree_root),);
    exec.run_command(machine_id, &add_cmd)
        .await
        .map_err(|e| format!("git add failed: {}", e))?;

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
