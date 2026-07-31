//! The pathspec `git add -A` runs with, and the one round trip that decides it.

use crate::domain::staged_deliverable::normalize_artifact_subdir;
use crate::paths;
use crate::ports::execution::ExecutionPort;

/// Probe the worktree for the exclusions the `git add` needs, in one round
/// trip, and return the pathspec suffix to append to `git add -A` (empty when
/// nothing is excluded).
///
/// Two independent rules, and a shared gate:
///
///   * `artifact_subdir`, when the caller opted out of committing artifacts.
///   * Any of `paths::DEPENDENCY_CACHE_DIRS` that is a symlink in *this*
///     worktree — i.e. one `provision_subtask_worktree` linked in from the
///     primary checkout. A symlink standing in for a directory isn't recognized
///     against a trailing-slash `.gitignore` pattern (see
///     `paths::DEPENDENCY_CACHE_DIRS`), so without the exclusion the symlink
///     itself — an absolute host path — gets staged and committed onto the
///     feature branch. Testing `-L` (rather than excluding the names
///     unconditionally) means a project that legitimately tracks a directory
///     sharing one of these names (e.g. Go's vendored `vendor/`) is unaffected
///     — we only ever skip our own symlinks, never a real tracked directory.
///
/// The shared gate is `check-ignore`: a candidate git already ignores must NOT
/// be excluded. Naming a path in a pathspec makes git treat it as explicitly
/// requested even when the pathspec is negative, so
/// `git add -A -- ':!node_modules'` fails outright ("The following paths are
/// ignored by one of your .gitignore files") whenever `node_modules` is
/// gitignored — which is the common case, since a `.gitignore` entry without a
/// trailing slash matches our symlink too. The exclusion is redundant for an
/// ignored path anyway (`git add -A` never stages one), so dropping it costs
/// nothing.
///
/// `; true` at the end matters: the loop's exit status would otherwise be that
/// of its last test, which is false (non-zero) whenever the final candidate
/// isn't excluded — `run_command` treats any non-zero exit as `Err` and the
/// whole exclusion list would be silently dropped.
///
/// A free function over the one port it needs, so the command it builds and the
/// answer it parses are both assertable against a double that answers this
/// probe and errors on anything else.
pub(crate) async fn resolve_add_exclusions(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    artifact_subdir: &str,
    commit_artifacts: bool,
) -> String {
    let trimmed = normalize_artifact_subdir(artifact_subdir);

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

    if exclusions.is_empty() {
        String::new()
    } else {
        format!(" --{exclusions}")
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/add_exclusions.rs"]
mod tests;
