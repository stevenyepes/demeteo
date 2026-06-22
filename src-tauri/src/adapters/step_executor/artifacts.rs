use std::sync::Arc;

use crate::domain::artifact::{Artifact, ArtifactCapture, ArtifactDecl, ArtifactSource};
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::paths;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::GateRepository;
use crate::ports::execution::ExecutionPort;

/// Returns `(decision, feedback)` from the most recently *decided* gate step
/// for a feature.  Used to inject `{{gate_decision}}` and `{{gate_feedback}}`
/// into the next agent step's rendered prompt.
///
/// Best-effort: returns `("", "")` when no gate has been decided yet (the
/// common case for the first agent step in any workflow).
pub(crate) fn get_latest_gate_decision(
    gates: &dyn GateRepository,
    feature_id: &str,
) -> (String, String) {
    let f_id = FeatureId::from(feature_id.to_string());
    match gates.latest_decided_for_feature(&f_id) {
        Ok(Some(decided)) => (
            decided.decision.unwrap_or_default(),
            decided.feedback.unwrap_or_default(),
        ),
        _ => (String::new(), String::new()),
    }
}

/// Resolve `[attached — <step_id>]` and `[attached — previous step artifact]`
/// placeholders inside a prompt template by reading the corresponding artifact
/// files from disk.
pub(crate) fn resolve_attached_artifacts(
    prompt: &str,
    step_execs: &[StepExecution],
    step_index: usize,
) -> String {
    let mut resolved_prompt = prompt.to_string();
    let mut search_start = 0;
    let mut attachments = Vec::new();

    while let Some(start_idx) = resolved_prompt[search_start..].find("[attached") {
        let absolute_start = search_start + start_idx;
        if let Some(end_offset) = resolved_prompt[absolute_start..].find(']') {
            let absolute_end = absolute_start + end_offset;
            let full_placeholder = resolved_prompt[absolute_start..=absolute_end].to_string();

            let inside = &full_placeholder[1..full_placeholder.len() - 1];

            let parts: Vec<&str> = if inside.contains('\u{2014}') {
                inside.split('\u{2014}').collect()
            } else if inside.contains('\u{2013}') {
                inside.split('\u{2013}').collect()
            } else {
                inside.split('-').collect()
            };

            if parts.len() >= 2 {
                let content = parts[1].trim();
                let mut replacement = String::new();

                if content == "previous step artifact" {
                    if step_index > 0 {
                        if let Some(prev_step) = step_execs.get(step_index - 1) {
                            let paths: Vec<&String> = if !prev_step.artifact_paths.is_empty() {
                                prev_step.artifact_paths.iter().collect()
                            } else {
                                prev_step.artifact_path.as_ref().into_iter().collect()
                            };
                            let mut parts_content = Vec::new();
                            for p in &paths {
                                match std::fs::read_to_string(p) {
                                    Ok(c) => parts_content.push(c),
                                    Err(_) => parts_content
                                        .push(format!("(Error reading artifact at {})", p)),
                                }
                            }
                            let art_content = if parts_content.len() == 1 {
                                parts_content.into_iter().next().unwrap_or_default()
                            } else {
                                parts_content.join("\n\n---\n\n")
                            };
                            attachments.push((
                                prev_step.step_index as usize,
                                prev_step.step_id.0.clone(),
                                art_content,
                            ));
                            replacement = format!(
                                "[See attached {} at the beginning of the prompt]",
                                prev_step.step_id.0
                            );
                        }
                    } else {
                        replacement = "(No previous step exists)".to_string();
                    }
                } else {
                    let mut found = false;
                    let mut matched_contents = Vec::new();
                    let mut matched_step_index = 0;
                    let mut matched_step_id = String::new();

                    for s in step_execs {
                        let sid = s.step_id.0.to_lowercase();
                        let content_lower = content.to_lowercase();

                        if content_lower.contains(&sid) || sid.contains(&content_lower) {
                            let paths: Vec<&String> = if !s.artifact_paths.is_empty() {
                                s.artifact_paths.iter().collect()
                            } else {
                                s.artifact_path.as_ref().into_iter().collect()
                            };
                            for p in &paths {
                                if let Ok(art_content) = std::fs::read_to_string(p) {
                                    matched_contents.push(art_content);
                                    matched_step_index = s.step_index as usize;
                                    matched_step_id = s.step_id.0.clone();
                                    found = true;
                                }
                            }
                        }
                    }

                    if found {
                        let art_content = matched_contents.join("\n\n");
                        attachments.push((
                            matched_step_index,
                            matched_step_id.clone(),
                            art_content,
                        ));
                        replacement = format!(
                            "[See attached {} at the beginning of the prompt]",
                            matched_step_id
                        );
                    } else {
                        replacement =
                            format!("(Artifact '{}' not found or not yet generated)", content);
                    }
                }

                resolved_prompt = resolved_prompt.replace(&full_placeholder, &replacement);
                search_start = 0;
                continue;
            }
        }
        search_start += start_idx + 1;
    }

    if !attachments.is_empty() {
        attachments.sort_by_key(|a| a.0);

        let mut prepended = String::new();
        for (_, step_id, content) in attachments {
            prepended.push_str(&format!(
                "=== ATTACHED CONTEXT: {} ===\n{}\n================================\n\n",
                step_id, content
            ));
        }
        resolved_prompt = format!("{}{}", prepended, resolved_prompt);
    }

    resolved_prompt
}

/// Append a synthetic `## Expected Artifacts (orchestrator contract)` block
/// to `prompt` when `declarations` is non-empty. The agent sees exactly
/// which named artifacts the orchestrator expects and where to write
/// them, without the prompt author having to repeat the contract in
/// natural-language prose.
///
/// Returns the original `prompt` unchanged when `declarations` is
/// `None` or empty (legacy backstop).
pub(crate) fn inject_artifact_contract(
    prompt: &str,
    declarations: Option<&[ArtifactDecl]>,
) -> String {
    let decls = match declarations {
        Some(d) if !d.is_empty() => d,
        _ => return prompt.to_string(),
    };

    let mut lines = vec![
        String::new(),
        "## Expected Artifacts (orchestrator contract)".to_string(),
        String::new(),
        "Capture your work in the following files so downstream".to_string(),
        "steps and the reviewer can see what you produced:".to_string(),
        String::new(),
    ];

    for d in decls {
        let hint = match &d.capture {
            ArtifactCapture::ByName { name } => {
                format!("- Produce an artifact named `{}`", name)
            }
            ArtifactCapture::LastWriteTo { path } => {
                format!("- Write `{}` → artifact `{}`", path, d.name)
            }
            ArtifactCapture::AllWrites => {
                "- Every file you write will be captured automatically via git".to_string()
            }
            ArtifactCapture::ChangedFiles { path_filter, .. } => {
                if let Some(filter) = path_filter {
                    format!(
                        "- All files matching `{}` will be captured automatically via git",
                        filter
                    )
                } else {
                    "- All changed files will be captured automatically via git".to_string()
                }
            }
            ArtifactCapture::Diff { .. } => {
                "- A diff will be computed at the end of the step".to_string()
            }
            ArtifactCapture::Worktree { path: Some(p) } => {
                format!("- Worktree pointer for `{}`", p)
            }
            ArtifactCapture::Worktree { path: None } => "- Worktree root pointer".to_string(),
        };
        lines.push(hint);
    }

    lines.push(String::new());
    lines.push(
        "Your file changes are automatically detected via git — no special naming required."
            .to_string(),
    );

    let mut result = prompt.to_string();
    result.push_str(&lines.join("\n"));
    result
}

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

/// A pre-step snapshot of the worktree's dirty state. The step
/// handler calls `WorktreeSnapshot::capture` at the *start* of a
/// step (before the agent runs), and `WorktreeSnapshot::delta` at
/// the *end*. The delta is the set of files this step's agent
/// actually created or modified, isolated from any state that
/// was already dirty from a prior step.
///
/// The snapshot is based on `git status --porcelain` rather than
/// commit boundaries, because the agent does not commit — it
/// only writes files into the worktree's working directory. A
/// commit-based snapshot would miss every file the agent wrote.
#[derive(Debug, Clone, Default)]
pub struct WorktreeSnapshot {
    /// Paths that were dirty at snapshot time, normalised to
    /// repo-relative form (e.g. `"research-report.md"`).
    dirty: std::collections::BTreeSet<String>,
}

impl WorktreeSnapshot {
    /// Capture a snapshot of `worktree_root`'s dirty state via
    /// `git status --porcelain`. If the worktree is not a git
    /// repo, or the command fails, returns an empty snapshot
    /// (the resulting delta is "everything currently dirty",
    /// which is a safe fallback for the bootstrap edge case).
    pub fn capture(exec: &dyn ExecutionPort, machine_id: &str, worktree_root: &str) -> Self {
        let dirty = parse_status_porcelain(&git_status_porcelain(exec, machine_id, worktree_root));
        Self { dirty }
    }

    /// Compute the file-level delta between this snapshot and the
    /// worktree's current state. Returns `Vec<rel_path>` for files
    /// that became dirty *during* this step (newly created, newly
    /// modified, newly staged, or newly deleted). Paths that were
    /// already dirty at snapshot time are excluded — those belong
    /// to a prior step.
    ///
    /// `always_include` lets the caller force-include specific
    /// paths (e.g. paths named in `ArtifactDecl::LastWriteTo`)
    /// even if they were dirty before the step started. This is
    /// how the orchestrator handles "refine the previous step's
    /// artifact" cases without losing the latest body.
    ///
    /// `.git/`, `.demeteo/`, and any path in `extra_exclude` are
    /// always filtered out — those are orchestrator scaffolding,
    /// not the agent's work.
    pub fn delta(
        &self,
        exec: &dyn ExecutionPort,
        machine_id: &str,
        worktree_root: &str,
        always_include: &[&str],
        extra_exclude: &[&str],
    ) -> Vec<String> {
        let now = parse_status_porcelain(&git_status_porcelain(exec, machine_id, worktree_root));

        let mut out: std::collections::BTreeSet<String> = now
            .iter()
            .filter(|p| !self.dirty.contains(p.as_str()))
            .cloned()
            .collect();

        for forced in always_include {
            if !forced.is_empty() {
                out.insert((*forced).to_string());
            }
        }

        out.into_iter()
            .filter(|p| !is_excluded(p, extra_exclude))
            .collect()
    }
}

fn git_status_porcelain(exec: &dyn ExecutionPort, machine_id: &str, worktree_root: &str) -> String {
    let cmd = format!(
        "git -C {} status --porcelain --untracked-files=all",
        paths::shell_escape_posix(worktree_root),
    );
    exec.run_command(machine_id, &cmd).unwrap_or_default()
}

/// Parse `git status --porcelain` output into a deduplicated set
/// of repo-relative paths. Renames appear as `R  old -> new`; we
/// keep `new` because that's the path the agent worked on. Lines
/// that look like branch info (those starting with `##`) are
/// dropped.
fn parse_status_porcelain(raw: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for line in raw.lines() {
        if line.is_empty() {
            continue;
        }
        // Branch info: "## main...origin/main [ahead 3]"
        if line.starts_with("##") {
            continue;
        }
        // Strip the leading 2-char XY status and the space.
        // Layout: "XY path" or "XY old -> new".
        if line.len() < 4 {
            continue;
        }
        let after_status = &line[3..];
        let path_part = if let Some(idx) = after_status.find(" -> ") {
            &after_status[idx + 4..]
        } else {
            after_status
        };
        // Unquote if necessary (git quotes paths with special chars).
        let path = path_part
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .map(unquote_git_path)
            .unwrap_or_else(|| path_part.to_string());
        if !path.is_empty() {
            out.insert(path);
        }
    }
    out
}

fn unquote_git_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(&next) = chars.peek() {
                chars.next();
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn is_excluded(path: &str, extra: &[&str]) -> bool {
    if path.starts_with(".git/") || path == ".git" {
        return true;
    }
    if path.starts_with(".demeteo/") || path == ".demeteo" {
        return true;
    }
    for ex in extra {
        if path == *ex {
            return true;
        }
    }
    false
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
pub fn read_worktree_file(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    rel_path: &str,
) -> Option<String> {
    let abs = format!("{}/{}", worktree_root.trim_end_matches('/'), rel_path);
    let content = exec.read_file(machine_id, &abs).ok()?;
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
pub fn compute_git_diff(
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
    exec.run_command(machine_id, &cmd).unwrap_or_default()
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
pub fn commit_worktree_changes(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    message: &str,
) -> Result<String, String> {
    let add_cmd = format!("git -C {} add -A", paths::shell_escape_posix(worktree_root),);
    exec.run_command(machine_id, &add_cmd)
        .map_err(|e| format!("git add failed: {}", e))?;

    let commit_cmd = format!(
        "git -C {} -c user.email=demeteo@local -c user.name=demeteo commit -m {} --allow-empty",
        paths::shell_escape_posix(worktree_root),
        paths::shell_escape_posix(message),
    );
    let out = exec
        .run_command(machine_id, &commit_cmd)
        .map_err(|e| format!("git commit failed: {}", e))?;

    let sha_cmd = format!(
        "git -C {} rev-parse HEAD",
        paths::shell_escape_posix(worktree_root),
    );
    exec.run_command(machine_id, &sha_cmd)
        .map(|s| s.trim().to_string())
        .map_err(|e| format!("git rev-parse after commit failed: {}", e))
        .inspect(|_sha| {
            // Best-effort: print the commit output so operators can see
            // what was committed. The orchestrator doesn't currently
            // surface this, but it's useful in the dev console.
            if !out.is_empty() {
                eprintln!("[commit_worktree_changes] {}", out.trim());
            }
        })
}

fn is_likely_binary(content: &str) -> bool {
    if content.contains('\0') {
        return true;
    }
    // Heuristic: if the first 8 KiB has no newlines and is "long enough"
    // it's probably a binary blob. Markdown / code / configs all contain
    // newlines; PNG/JPG/zlib blobs do not.
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
mod tests {
    use super::*;
    use crate::domain::ids::FeatureId;
    use crate::domain::ids::StepExecutionId;

    #[test]
    fn test_resolve_attached_artifacts() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_artifacts_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let path1 = temp_dir.join("s-spec.md");
        std::fs::write(&path1, "This is the spec content.").unwrap();
        let path1_str = path1.to_string_lossy().to_string();

        let path2 = temp_dir.join("s-research.md");
        std::fs::write(&path2, "This is the research content.").unwrap();
        let path2_str = path2.to_string_lossy().to_string();

        let step_execs = vec![
            StepExecution {
                id: StepExecutionId::from("se-1"),
                feature_id: FeatureId::from("f-1"),
                step_id: crate::domain::ids::StepId::from("s-research"),
                step_index: 0,
                step_kind: "agent".to_string(),
                status: "completed".to_string(),
                cost_usd: Some(0.0),
                wall_clock_secs: Some(0),
                artifact_path: Some(path2_str),
                artifact_paths: vec![],
                error_message: None,
                iteration_count: 0,
                created_at: 0,
                updated_at: 0,
            },
            StepExecution {
                id: StepExecutionId::from("se-2"),
                feature_id: FeatureId::from("f-1"),
                step_id: crate::domain::ids::StepId::from("s-spec"),
                step_index: 1,
                step_kind: "agent".to_string(),
                status: "completed".to_string(),
                cost_usd: Some(0.0),
                wall_clock_secs: Some(0),
                artifact_path: Some(path1_str),
                artifact_paths: vec![],
                error_message: None,
                iteration_count: 0,
                created_at: 0,
                updated_at: 0,
            },
        ];

        let template =
            "Read the research: [attached — s-research] and the spec: [attached — s-spec]";
        let resolved = resolve_attached_artifacts(template, &step_execs, 1);
        assert_eq!(
            resolved,
            "=== ATTACHED CONTEXT: s-research ===\nThis is the research content.\n================================\n\n=== ATTACHED CONTEXT: s-spec ===\nThis is the spec content.\n================================\n\nRead the research: [See attached s-research at the beginning of the prompt] and the spec: [See attached s-spec at the beginning of the prompt]"
        );

        let template_prev = "Previous content: [attached — previous step artifact]";
        let resolved_prev = resolve_attached_artifacts(template_prev, &step_execs, 1);
        assert_eq!(
            resolved_prev,
            "=== ATTACHED CONTEXT: s-research ===\nThis is the research content.\n================================\n\nPrevious content: [See attached s-research at the beginning of the prompt]"
        );

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_inject_artifact_contract_empty() {
        let prompt = "Do the thing.";
        let result = inject_artifact_contract(prompt, None);
        assert_eq!(result, prompt);

        let result = inject_artifact_contract(prompt, Some(&[]));
        assert_eq!(result, prompt);
    }

    #[test]
    fn test_inject_artifact_contract_with_decls() {
        let prompt = "Write the spec.";
        let decls = vec![ArtifactDecl::full_path("spec", "docs/spec.md")];
        let result = inject_artifact_contract(prompt, Some(&decls));
        assert!(result.contains("## Expected Artifacts (orchestrator contract)"));
        assert!(result.contains("Write `docs/spec.md`"));
        assert!(result.contains("artifact `spec`"));
        assert!(result.starts_with("Write the spec."));
    }

    #[test]
    fn test_inject_artifact_contract_all_capture_kinds() {
        use crate::domain::artifact::DiffBase;
        let prompt = "Implement everything.";
        let decls = vec![
            ArtifactDecl::full_path("spec", "docs/spec.md"),
            ArtifactDecl {
                name: "impl".into(),
                capture: ArtifactCapture::AllWrites,
                mode: crate::domain::artifact::ArtifactMode::Full,
            },
            ArtifactDecl {
                name: "diff".into(),
                capture: ArtifactCapture::Diff {
                    base: DiffBase::WorktreeBase,
                    path_filter: None,
                },
                mode: crate::domain::artifact::ArtifactMode::Full,
            },
            ArtifactDecl {
                name: "wt".into(),
                capture: ArtifactCapture::Worktree {
                    path: Some("src/".into()),
                },
                mode: crate::domain::artifact::ArtifactMode::None,
            },
        ];
        let result = inject_artifact_contract(prompt, Some(&decls));
        assert!(result.contains("Write `docs/spec.md`"));
        assert!(result.contains("Every file you write will be captured"));
        assert!(result.contains("A diff will be computed"));
        assert!(result.contains("Worktree pointer for `src/`"));
    }

    #[test]
    fn test_resolve_declared_artifacts_by_name() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_name_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> = Arc::new(
            crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
        );

        let declarations = vec![ArtifactDecl::full_path("spec", "docs/spec.md")];

        let produced = vec![Artifact::tool_write("spec", "docs/spec.md", "# My Spec\n")];

        let refs = resolve_declared_artifacts(&declarations, &produced, &store, "f-test", "s-impl");

        assert_eq!(refs.len(), 1);
        assert!(refs[0].contains("artifacts/f-test/s-impl/spec"));
        // Verify content was stored
        let content = store.get(&refs[0]).unwrap();
        assert_eq!(content, "# My Spec\n");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_declared_artifacts_last_write() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_last_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> = Arc::new(
            crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
        );

        let declarations = vec![ArtifactDecl {
            name: "final-spec".into(),
            capture: ArtifactCapture::LastWriteTo {
                path: "docs/spec.md".into(),
            },
            mode: crate::domain::artifact::ArtifactMode::Full,
        }];

        let produced = vec![
            Artifact::tool_write("draft", "docs/spec.md", "# Draft\n"),
            Artifact::tool_write("final", "docs/spec.md", "# Final\n"),
        ];

        let refs = resolve_declared_artifacts(&declarations, &produced, &store, "f-test", "s-impl");

        assert_eq!(refs.len(), 1);
        let content = store.get(&refs[0]).unwrap();
        assert_eq!(content, "# Final\n");

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_declared_artifacts_all_writes() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_all_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> = Arc::new(
            crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
        );

        let declarations = vec![ArtifactDecl {
            name: "all-files".into(),
            capture: ArtifactCapture::AllWrites,
            mode: crate::domain::artifact::ArtifactMode::Full,
        }];

        let produced = vec![
            Artifact::tool_write("f1", "src/lib.rs", "// lib\n"),
            Artifact::tool_write("f2", "src/main.rs", "// main\n"),
            // duplicate path should be deduplicated
            Artifact::tool_write("f1-v2", "src/lib.rs", "// lib v2\n"),
        ];

        let refs = resolve_declared_artifacts(&declarations, &produced, &store, "f-test", "s-impl");

        // Two unique paths: src/lib.rs (last write wins for content, but ref deduped)
        assert_eq!(refs.len(), 2);

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_declared_artifacts_skips_diff_and_worktree() {
        use crate::domain::artifact::DiffBase;
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_resolve_skip_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let store: Arc<dyn ArtifactStore> = Arc::new(
            crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
        );

        let declarations = vec![
            ArtifactDecl {
                name: "code-diff".into(),
                capture: ArtifactCapture::Diff {
                    base: DiffBase::WorktreeBase,
                    path_filter: None,
                },
                mode: crate::domain::artifact::ArtifactMode::Full,
            },
            ArtifactDecl {
                name: "wt-ref".into(),
                capture: ArtifactCapture::Worktree { path: None },
                mode: crate::domain::artifact::ArtifactMode::None,
            },
        ];

        // Produced has no matching artifact — diff/worktree are derived
        let refs = resolve_declared_artifacts(&declarations, &[], &store, "f-test", "s-impl");

        assert!(refs.is_empty());

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn test_resolve_attached_artifacts_uses_artifact_paths() {
        let temp_dir = std::env::temp_dir().join(format!(
            "demeteo_test_attach_paths_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        std::fs::create_dir_all(&temp_dir).unwrap();

        let artifact_file = temp_dir.join("s-research.md");
        std::fs::write(&artifact_file, "Research content from paths.").unwrap();
        let artifact_str = artifact_file.to_string_lossy().to_string();

        let step_execs = vec![StepExecution {
            id: StepExecutionId::from("se-1"),
            feature_id: FeatureId::from("f-1"),
            step_id: crate::domain::ids::StepId::from("s-research"),
            step_index: 0,
            step_kind: "agent".to_string(),
            status: "completed".to_string(),
            cost_usd: Some(0.0),
            wall_clock_secs: Some(0),
            artifact_path: None,
            artifact_paths: vec![artifact_str],
            error_message: None,
            iteration_count: 0,
            created_at: 0,
            updated_at: 0,
        }];

        let template = "Previous: [attached — previous step artifact]";
        let resolved = resolve_attached_artifacts(template, &step_execs, 1);
        assert_eq!(
            resolved,
            "=== ATTACHED CONTEXT: s-research ===\nResearch content from paths.\n================================\n\nPrevious: [See attached s-research at the beginning of the prompt]"
        );

        let _ = std::fs::remove_dir_all(temp_dir);
    }

    // ── WorktreeSnapshot & helpers ──────────────────────────────────

    #[test]
    fn test_parse_status_porcelain_basic() {
        let raw = "?? untracked.md\n";
        let set = parse_status_porcelain(raw);
        assert!(set.contains("untracked.md"));

        let raw_mod = " M modified.rs\n";
        let set = parse_status_porcelain(raw_mod);
        assert!(set.contains("modified.rs"));

        let raw_rename = "R  old.txt -> new.txt\n";
        let set = parse_status_porcelain(raw_rename);
        assert!(set.contains("new.txt"));
        assert!(!set.contains("old.txt"));

        // Branch info line is dropped
        let raw_branch = "## main...origin/main\n";
        let set = parse_status_porcelain(raw_branch);
        assert!(set.is_empty());
    }

    #[test]
    fn test_parse_status_porcelain_dedup() {
        let raw = "?? dup.md\n?? dup.md\n";
        let set = parse_status_porcelain(raw);
        assert_eq!(set.len(), 1);
    }

    #[test]
    fn test_snapshot_delta_detects_new_files() {
        let temp = temp_git_repo("snapshot_delta_new");
        let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
        let machine = "local";

        // Write & commit a baseline file so the repo isn't empty.
        exec.write_file(machine, &format!("{}/baseline.rs", temp), "fn main() {}")
            .unwrap();
        exec.run_command(
            machine,
            &format!(
                "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m init",
                shell_esc(&temp),
                shell_esc(&temp),
            ),
        )
        .unwrap();

        // Snapshot the clean repo.
        let snap = WorktreeSnapshot::capture(&exec, machine, &temp);
        assert!(snap.dirty.is_empty());

        // Simulate the agent writing a new file and modifying
        // baseline.rs.
        exec.write_file(machine, &format!("{}/new.md", temp), "# New\n")
            .unwrap();
        exec.write_file(machine, &format!("{}/baseline.rs", temp), "fn main(){}\n")
            .unwrap();

        // Delta with always_include empty: the new file should appear.
        let changed = snap.delta(&exec, machine, &temp, &[], &[]);
        assert!(
            changed.contains(&"new.md".to_string()),
            "expected new.md in delta, got {:?}",
            changed
        );
        // baseline.rs was clean *before* the step and is now modified.
        // `git status --porcelain` will report it as " M" so it's dirty now.
        assert!(
            changed.contains(&"baseline.rs".to_string()),
            "expected baseline.rs in delta (modified by step), got {:?}",
            changed
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_snapshot_delta_always_include() {
        let temp = temp_git_repo("snapshot_delta_always");
        let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
        let machine = "local";

        exec.write_file(machine, &format!("{}/base.md", temp), "# base\n")
            .unwrap();
        exec.run_command(
            machine,
            &format!(
                "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m init",
                shell_esc(&temp),
                shell_esc(&temp),
            ),
        )
        .unwrap();

        // Make base.md dirty before the step starts.
        exec.write_file(machine, &format!("{}/base.md", temp), "# dirty\n")
            .unwrap();
        let snap = WorktreeSnapshot::capture(&exec, machine, &temp);
        assert!(snap.dirty.contains("base.md"));

        // Step refines base.md further.
        exec.write_file(machine, &format!("{}/base.md", temp), "# final\n")
            .unwrap();

        // Without always_include, base.md is excluded because it was
        // already dirty at step start.
        let without = snap.delta(&exec, machine, &temp, &[], &[]);
        assert!(
            !without.contains(&"base.md".to_string()),
            "base.md should NOT appear without always_include, got {:?}",
            without
        );

        // With always_include = ["base.md"], it appears regardless.
        let with = snap.delta(&exec, machine, &temp, &["base.md"], &[]);
        assert!(
            with.contains(&"base.md".to_string()),
            "base.md should appear with always_include, got {:?}",
            with
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_snapshot_delta_excludes_scaffolding() {
        let temp = temp_git_repo("snapshot_exclude");
        let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
        let machine = "local";

        exec.write_file(machine, &format!("{}/base.md", temp), "# b\n")
            .unwrap();
        exec.run_command(
            machine,
            &format!(
                "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m init",
                shell_esc(&temp),
                shell_esc(&temp),
            ),
        )
        .unwrap();

        let snap = WorktreeSnapshot::capture(&exec, machine, &temp);

        // Write scaffolding files that the delta should filter.
        std::fs::create_dir_all(format!("{}/.git/tmp", temp)).unwrap();
        std::fs::write(format!("{}/.git/tmp/x", temp), "x").unwrap();
        std::fs::create_dir_all(format!("{}/.demeteo/data", temp)).unwrap();
        std::fs::write(format!("{}/.demeteo/data/y", temp), "y").unwrap();

        let changed = snap.delta(&exec, machine, &temp, &[], &[]);
        assert!(
            !changed.iter().any(|p| p.starts_with(".git")),
            "should exclude .git paths, got {:?}",
            changed
        );
        assert!(
            !changed.iter().any(|p| p.starts_with(".demeteo")),
            "should exclude .demeteo paths, got {:?}",
            changed
        );

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_commit_worktree_changes() {
        let temp = temp_git_repo("commit_worktree");
        let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
        let machine = "local";

        exec.write_file(machine, &format!("{}/src.rs", temp), "fn a() {}\n")
            .unwrap();
        exec.run_command(
            machine,
            &format!(
                "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m base",
                shell_esc(&temp),
                shell_esc(&temp),
            ),
        )
        .unwrap();

        // Modify a tracked file and add an untracked one — both should
        // be committed by commit_worktree_changes.
        exec.write_file(machine, &format!("{}/src.rs", temp), "fn b() {}\n")
            .unwrap();
        exec.write_file(machine, &format!("{}/new.md", temp), "# Added\n")
            .unwrap();

        let sha = commit_worktree_changes(&exec, machine, &temp, "worker: subtask-1").unwrap();
        assert!(!sha.is_empty());

        // Verify the commit exists and the tree changed.
        let log = exec
            .run_command(
                machine,
                &format!("git -C {} log --oneline -1", shell_esc(&temp)),
            )
            .unwrap();
        assert!(log.contains("worker: subtask-1"));

        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn test_compute_git_diff() {
        let temp = temp_git_repo("compute_diff");
        let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
        let machine = "local";

        exec.write_file(machine, &format!("{}/src.rs", temp), "fn init() {}\n")
            .unwrap();
        exec.run_command(
            machine,
            &format!(
                "git -C {} add -A && git -c user.email=t@t.com -c user.name=t -C {} commit -m base",
                shell_esc(&temp),
                shell_esc(&temp),
            ),
        )
        .unwrap();

        let base_sha = exec
            .run_command(
                machine,
                &format!("git -C {} rev-parse HEAD", shell_esc(&temp)),
            )
            .unwrap()
            .trim()
            .to_string();

        // Modify the file but don't commit.
        exec.write_file(machine, &format!("{}/src.rs", temp), "fn new() {}\n")
            .unwrap();

        // Diff against the initial commit.
        let diff = compute_git_diff(&exec, machine, &temp, &base_sha);
        assert!(!diff.is_empty());
        assert!(diff.contains("fn init()"));
        assert!(diff.contains("fn new()"));

        // Diff against HEAD — should still show uncommitted changes.
        let diff_head = compute_git_diff(&exec, machine, &temp, "HEAD");
        assert!(!diff_head.is_empty());

        // Diff against a nonexistent ref — should return empty.
        let diff_none = compute_git_diff(&exec, machine, &temp, "no-such-ref");
        assert!(diff_none.is_empty());

        let _ = std::fs::remove_dir_all(&temp);
    }

    // ── Helpers ─────────────────────────────────────────────────────

    fn temp_git_repo(label: &str) -> String {
        let d = std::env::temp_dir().join(format!(
            "demeteo_test_{}_{}",
            label,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis(),
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let exec = crate::adapters::local::execution::LocalSubprocessAdapter::new();
        let _ = exec.run_command(
            "local",
            &format!("git init -b main {}", shell_esc(&d.to_string_lossy()),),
        );
        d.to_string_lossy().to_string()
    }

    fn shell_esc(s: &str) -> String {
        crate::paths::shell_escape_posix(s)
    }
}
