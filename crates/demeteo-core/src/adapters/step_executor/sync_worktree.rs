//! Tearing down the throwaway worktree a conflict resolution ran in.

use crate::paths;
use crate::ports::execution::ExecutionPort;

/// Remove the sync worktree, if one was used.
///
/// The `worktree != repo_dir` guard is **inside** rather than at the call
/// site: what it prevents is an `rm -rf` over the main repo checkout, and it
/// used to be spelled separately by each of the two callers — a shape where
/// the third caller is the one that forgets.
///
/// Every command is best-effort. A worktree that will not unregister must
/// still be deleted, and a stale entry must still be pruned, so a failure in
/// one step does not stop the next.
pub(crate) async fn discard_sync_worktree(
    exec: &dyn ExecutionPort,
    machine: &str,
    repo_dir: &str,
    worktree: &str,
) {
    if worktree == repo_dir {
        return;
    }
    let _ = exec
        .run_command(
            machine,
            &format!(
                "git -C {} worktree remove --force {}",
                paths::shell_escape_posix(repo_dir),
                paths::shell_escape_posix(worktree)
            ),
        )
        .await;
    let _ = exec
        .run_command(
            machine,
            &format!("rm -rf {}", paths::shell_escape_posix(worktree)),
        )
        .await;
    let _ = exec
        .run_command(
            machine,
            &format!(
                "git -C {} worktree prune",
                paths::shell_escape_posix(repo_dir)
            ),
        )
        .await;
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/sync_worktree.rs"]
mod tests;
