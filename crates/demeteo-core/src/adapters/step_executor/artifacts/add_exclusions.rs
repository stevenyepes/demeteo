//! The pathspec `git add -A` runs with, and the one round trip that decides it.

use crate::domain::staged_deliverable::normalize_artifact_subdir;
use crate::paths;
use crate::ports::execution::ExecutionPort;

/// Probe the worktree for the exclusions the `git add` needs, in one round
/// trip, and return the pathspec suffix to append to `git add -A` (empty when
/// nothing is excluded).
///
/// One rule is left: `artifact_subdir`, when the caller opted out of committing
/// artifacts.
///
/// # The dependency caches are not decided here any more
///
/// They used to be — every entry of `paths::DEPENDENCY_CACHE_DIRS` that was a
/// symlink in *this* worktree was pathspec-excluded, because a symlink standing
/// in for a directory is not matched by a trailing-slash `.gitignore` pattern
/// and would otherwise be staged as an absolute host path. That answer is now
/// written once into the clone's `.git/info/exclude` at provisioning time
/// (`git_ops::worktree::exclude_file_with`), which makes the symlink ignored
/// outright, so a pathspec naming it would be both redundant and — per the gate
/// below — refused.
///
/// The gate that remains is `check-ignore`: a candidate git already ignores
/// must NOT be excluded. Naming a path in a pathspec makes git treat it as
/// explicitly requested even when the pathspec is negative, so
/// `git add -A -- ':!artifacts'` fails outright ("The following paths are
/// ignored by one of your .gitignore files") whenever the project gitignores
/// its artifact directory. The exclusion is redundant for an ignored path
/// anyway (`git add -A` never stages one), so dropping it costs nothing.
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
    if commit_artifacts || trimmed.is_empty() {
        return String::new();
    }

    let escaped = paths::shell_escape_posix(trimmed);
    // `cd` guards with `|| exit 1` rather than `&&`: the `;`-separated probe
    // that follows would otherwise still run, in the wrong directory, and
    // report an exclusion for the wrong repo.
    //
    // `check-ignore -q` exits 0 when ignored and 1 when not; only a definite 1
    // clears the candidate, so an error exit (128) leaves it out rather than
    // reintroducing the failure above. `; true` closes the command because that
    // `[ $? -eq 1 ]` is its last test.
    let exclusion_probe = format!(
        "cd {wt} || exit 1; git check-ignore -q -- {escaped} 2>/dev/null; \
         [ $? -eq 1 ] && echo {escaped}; true",
        wt = paths::shell_escape_posix(worktree_root),
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
            exclusions.push_str(&format!(" ':!{trimmed}'"));
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
