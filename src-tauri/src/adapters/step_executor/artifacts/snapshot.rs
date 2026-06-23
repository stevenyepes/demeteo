use crate::paths;
use crate::ports::execution::ExecutionPort;

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
    pub async fn capture(exec: &dyn ExecutionPort, machine_id: &str, worktree_root: &str) -> Self {
        let dirty =
            parse_status_porcelain(&git_status_porcelain(exec, machine_id, worktree_root).await);
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
    pub async fn delta(
        &self,
        exec: &dyn ExecutionPort,
        machine_id: &str,
        worktree_root: &str,
        always_include: &[&str],
        extra_exclude: &[&str],
    ) -> Vec<String> {
        let now =
            parse_status_porcelain(&git_status_porcelain(exec, machine_id, worktree_root).await);

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

async fn git_status_porcelain(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
) -> String {
    let cmd = format!(
        "git -C {} status --porcelain --untracked-files=all",
        paths::shell_escape_posix(worktree_root),
    );
    exec.run_command(machine_id, &cmd).await.unwrap_or_default()
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

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/snapshot.rs"]
mod tests;
