//! Whether a resolved worktree still has a commit owed to it.
//!
//! Both conflict flows — the merge-back pass and the sync resolver — end by
//! committing what the agent resolved, and both are told not to commit it
//! themselves. Agents do it anyway, and often: told to fix conflict markers, an
//! agent very commonly stages and commits on its own. That consumes
//! `MERGE_HEAD` and leaves a clean tree, so an unconditional `git commit` exits
//! non-zero with "nothing to commit" and the caller fails a merge that in fact
//! succeeded. A clean tree with the conflicts gone *is* the success condition.
//!
//! The probe lives here rather than in either caller because the second caller
//! was written without it and shipped that exact bug.

use crate::paths;
use crate::ports::execution::ExecutionPort;

/// Is there anything for `git commit` to record in `wt_path` — either an
/// in-progress merge to conclude, or modified tracked files?
///
/// `git status --porcelain` is empty exactly when the tree is clean, and
/// `MERGE_HEAD` exists exactly while a merge is awaiting its commit. An
/// agent that resolved *and committed* leaves neither.
pub(crate) async fn worktree_has_pending_commit(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    wt_path: &str,
) -> bool {
    let safe = paths::shell_escape_posix(wt_path);
    let merge_in_progress = exec
        .run_command(
            machine_str,
            &format!("git -C {} rev-parse --verify --quiet MERGE_HEAD", safe),
        )
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false);
    if merge_in_progress {
        return true;
    }
    exec.run_command(machine_str, &format!("git -C {} status --porcelain", safe))
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/steps/pending_commit.rs"]
mod tests;
