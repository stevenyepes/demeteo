//! Declared-artifact capture (git snapshot/diff/commit of a step's output).
//!
//! **Shell-context audit (C1.3, `docs/EXECUTION_PARITY.md`).** The
//! `exec.run_command` calls in this module all shell out to the system `git`
//! binary with an explicit `git -C <worktree>` cwd, so the non-login default
//! [`ShellOptions`](crate::ports::execution::ShellOptions) is intended: `git`
//! is on the default `PATH` of both a local `sh -c` and a bare SSH channel, and
//! no login profile is consulted, so local and SSH capture identically (D2). No
//! call here relies on ambient env or ambient cwd.
use crate::domain::artifact::{Artifact, ArtifactDecl, ArtifactSource};
use crate::paths;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::execution::ExecutionPort;
use std::sync::Arc;

// Re-exported rather than re-homed: they are decisions about what a step
// delivered, not I/O, so they live in `domain::artifact_capture` beside the
// message that fails the step.
use crate::domain::artifact_capture::{
    all_writes_selection, resolve_capture, CaptureOutcome, UnwiredCapture,
};
pub(crate) use crate::domain::artifact_capture::{note_undelivered_artifacts, MissingArtifact};

/// Resolve `declarations` against the `ArtifactProduced` events emitted
/// by the agent during a step turn. Writes matching artifacts through
/// the store and returns `(refs, missing)`:
/// * `refs` — the list of references (paths for the FS adapter) to
///   persist in `StepExecution.artifact_paths`.
/// * `missing` — the `ByName` / `LastWriteTo` declarations that matched
///   no produced artifact. These are the step's declared *deliverables*;
///   a non-empty `missing` means the agent ran but did not produce what
///   the workflow contract requires, so the caller fails the step (the
///   deliverable is the whole point of the step). Catch-all captures
///   (`AllWrites`, `ChangedFiles`, `Diff`, `Worktree`) are never counted
///   as missing — an empty result is legitimate for them.
pub(crate) fn resolve_declared_artifacts(
    declarations: &[ArtifactDecl],
    produced: &[Artifact],
    store: &Arc<dyn ArtifactStore>,
    feature_id: &str,
    step_id: &str,
) -> (Vec<String>, Vec<MissingArtifact>) {
    let mut refs = Vec::new();
    let mut missing = Vec::new();

    for decl in declarations {
        match resolve_capture(decl, produced) {
            CaptureOutcome::Store(artifact) => match store.put(feature_id, step_id, artifact) {
                Ok(reference) => refs.push(reference),
                Err(e) => {
                    tracing::warn!(
                        step = %step_id,
                        decl = %decl.name,
                        error = %e,
                        "resolve_declared_artifacts: failed to store artifact",
                    );
                }
            },
            CaptureOutcome::Skip(None) => {}
            // The orchestrator should synthesise these at TurnComplete when
            // `GitOpsHelper` methods are available (next step).
            CaptureOutcome::Skip(Some(UnwiredCapture::Diff)) => {
                eprintln!(
                    "[artifacts] step={} decl={}: Diff declaration skipped — GitOpsHelper not yet wired",
                    step_id, decl.name,
                );
            }
            CaptureOutcome::Skip(Some(UnwiredCapture::Worktree)) => {
                eprintln!(
                    "[artifacts] step={} decl={}: Worktree declaration skipped — GitOpsHelper not yet wired",
                    step_id, decl.name,
                );
            }
            CaptureOutcome::Missing(entry) => {
                tracing::warn!(
                    step = %step_id,
                    decl = %decl.name,
                    capture = ?decl.capture,
                    "resolve_declared_artifacts: no ArtifactProduced event matched this declaration — failing the step",
                );
                missing.push(entry);
            }
        }
    }

    for artifact in all_writes_selection(declarations, produced) {
        match store.put(feature_id, step_id, artifact) {
            Ok(reference) => refs.push(reference),
            Err(e) => {
                if let ArtifactSource::ToolWrite { path } = &artifact.source {
                    eprintln!(
                        "[artifacts] step={} path={}: Failed to store AllWrites artifact: {}",
                        step_id, path, e,
                    );
                }
            }
        }
    }

    (refs, missing)
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
    // D3: surface the read failure instead of swallowing it. A declared
    // artifact path that can't be read is exactly the "silent no-artifacts
    // on SSH" bug — the read errored (e.g. the remote file was never
    // produced, or the transport dropped) and the old `.ok()?` turned that
    // into a green step with an empty artifact. We still return `None`
    // (there is genuinely nothing to capture), but now it is observable.
    let content = match exec.read_file(machine_id, &abs).await {
        Ok(content) => content,
        Err(e) => {
            tracing::warn!(
                machine_id = %machine_id,
                path = %abs,
                error = %e,
                "read_worktree_file: declared artifact path could not be read — capturing nothing for it",
            );
            return None;
        }
    };
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
    // D3: an empty diff and a *failed* diff are not the same thing. The old
    // `.unwrap_or_default()` collapsed both to `""`, so a broken worktree or
    // an unresolvable `base_ref` looked identical to "no changes". Keep the
    // empty-string fallback (callers treat it as "no diff") but surface the
    // failure so it is diagnosable.
    match exec.run_command(machine_id, &cmd).await {
        Ok(diff) => diff,
        Err(e) => {
            tracing::warn!(
                machine_id = %machine_id,
                worktree = %worktree_root,
                base_ref = %base_ref,
                error = %e,
                "compute_git_diff: git diff failed — treating as an empty diff",
            );
            String::new()
        }
    }
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

    // Probe the worktree for the exclusions the `git add` needs, in one
    // round trip. Two independent rules, and a shared gate:
    //
    //   * `artifact_subdir`, when the caller opted out of committing
    //     artifacts.
    //   * Any of `paths::DEPENDENCY_CACHE_DIRS` that is a symlink in
    //     *this* worktree — i.e. one `provision_subtask_worktree` linked
    //     in from the primary checkout. A symlink standing in for a
    //     directory isn't recognized against a trailing-slash
    //     `.gitignore` pattern (see `paths::DEPENDENCY_CACHE_DIRS`), so
    //     without the exclusion the symlink itself — an absolute host
    //     path — gets staged and committed onto the feature branch.
    //     Testing `-L` (rather than excluding the names unconditionally)
    //     means a project that legitimately tracks a directory sharing
    //     one of these names (e.g. Go's vendored `vendor/`) is
    //     unaffected — we only ever skip our own symlinks, never a real
    //     tracked directory.
    //
    // The shared gate is `check-ignore`: a candidate git already ignores
    // must NOT be excluded. Naming a path in a pathspec makes git treat
    // it as explicitly requested even when the pathspec is negative, so
    // `git add -A -- ':!node_modules'` fails outright ("The following
    // paths are ignored by one of your .gitignore files") whenever
    // `node_modules` is gitignored — which is the common case, since a
    // `.gitignore` entry without a trailing slash matches our symlink
    // too. The exclusion is redundant for an ignored path anyway (`git
    // add -A` never stages one), so dropping it costs nothing.
    //
    // `; true` at the end matters: the loop's exit status would
    // otherwise be that of its last test, which is false (non-zero)
    // whenever the final candidate isn't excluded — `run_command` treats
    // any non-zero exit as `Err` and the whole exclusion list would be
    // silently dropped.
    let not_ignored = |p: &str| {
        // `check-ignore -q` exits 0 when ignored and 1 when not; only a
        // definite 1 clears a candidate for exclusion, so an error exit
        // (128) leaves it out rather than reintroducing the failure above.
        format!(
            "git check-ignore -q -- {p} 2>/dev/null; [ $? -eq 1 ] && echo {p}",
            p = p,
        )
    };
    let artifact_probe = if !commit_artifacts && !trimmed.is_empty() {
        format!("{}; ", not_ignored(&paths::shell_escape_posix(trimmed)))
    } else {
        String::new()
    };
    // `cd` guards with `|| exit 1` rather than `&&`: the `;`-separated
    // probes that follow would otherwise still run, in the wrong
    // directory, and report exclusions for the wrong repo.
    let exclusion_probe = format!(
        "cd {wt} || exit 1; {artifact_probe}for d in {dirs}; do [ -L \"$d\" ] && {{ {gate}; }}; done; true",
        wt = paths::shell_escape_posix(worktree_root),
        dirs = crate::paths::DEPENDENCY_CACHE_DIRS.join(" "),
        gate = not_ignored("\"$d\""),
    );
    let mut exclusions = String::new();
    match exec.run_command(machine_id, &exclusion_probe).await {
        Ok(out) => {
            for name in out.lines().map(str::trim).filter(|s| !s.is_empty()) {
                exclusions.push_str(&format!(" ':!{name}'"));
            }
        }
        Err(e) => {
            // The probe realistically only fails when the transport is
            // gone, in which case the `git add` below fails too and the
            // step surfaces that instead. Keep the artifact exclusion so
            // a probe failure can never quietly commit reports into the
            // PR.
            tracing::warn!(
                worktree = %worktree_root,
                error = %e,
                "commit_worktree_changes: exclusion probe failed; falling back to the artifact exclusion alone",
            );
            if !commit_artifacts && !trimmed.is_empty() {
                exclusions.push_str(&format!(" ':!{trimmed}'"));
            }
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
                // Fail the step instead of silently producing a commit
                // that contains only the summary report. The
                // historical docs-update bug was exactly this: the
                // agent emitted the real doc body under
                // `artifacts/s-draft.md` instead of `docs/<area>/<topic>.md`,
                // and the orchestrator happily committed the summary
                // report (or, with `commit_artifacts=false`, silently
                // produced an empty commit). Surfacing this as a
                // `StepOutcome::Failed` lets the retry loop feed the
                // failure reason back into `{{retry_feedback}}` so the
                // next attempt is directed at the real repo path.
                // See d9dcd53 for the original warn-only behaviour and
                // the docs-update workflow's two-distinct-outputs
                // prompt for the agent-side mitigation.
                let reason = format!(
                    "agent stranded the deliverable under `{}` instead of writing it to \
                     the real repo path. Stage contains only artifact paths \
                     ({:?}) while the agent reported writes outside the report subdir \
                     ({:?}). Re-read the survey's 'Files to Create' / 'Files to Update' \
                     sections and write the doc body to the real repo path (e.g. \
                     `docs/<area>/<topic>.md`), NOT to {}/s-*.md.",
                    trimmed_subdir, staged, non_artifact_writes, trimmed_subdir,
                );
                tracing::error!(
                    worktree = %worktree_root,
                    message = %message,
                    expected_non_artifact_writes = ?non_artifact_writes,
                    staged = ?staged,
                    "commit_worktree_changes: agent stranded the deliverable under `{}` \
                     — failing the step so the retry loop can redirect the agent to the \
                     real repo path",
                    artifact_subdir,
                );
                return Err(reason);
            }
        }
    }

    let commit_cmd = format!(
        "{} -c user.email=demeteo@local -c user.name=demeteo commit -m {} --allow-empty",
        paths::git_no_hooks(worktree_root),
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
    // Slice at a UTF-8 char boundary at or below 8 KiB. Indexing a `String`
    // directly at `content.len().min(8192)` panics when byte 8192 lands in
    // the middle of a multibyte char (common for the large UTF-8 reports
    // this capture path exists to handle) — walk back to the nearest
    // boundary instead.
    let head = head_str(content, 8192);
    if head.len() > 256 && !head.contains('\n') {
        return true;
    }
    false
}

/// Borrow the leading `max` bytes of `s`, truncated down to the nearest
/// UTF-8 char boundary so it never panics on a multibyte char straddling
/// `max`. Returns all of `s` when it is shorter than `max`.
fn head_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/declared.rs"]
mod tests;
