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
use crate::ports::execution::{ask, Answer, ExecutionPort};

/// What `git` said about `wt_path` having a commit owed to it.
///
/// Three arms rather than a `bool` because the caller's stakes are asymmetric:
/// [`Nothing`](Self::Nothing) skips the commit, and the sync resolver then
/// pushes a no-op, reads the pre-merge sha back as its result, files the
/// session `Resolved` and force-removes the worktree the resolution lived in.
/// A per-command timeout collapsed into "nothing to commit" is therefore
/// enough to destroy an agent's work while telling the user it landed, which is
/// why an unanswered probe is its own arm and never a negative
/// ([`Answer`](crate::ports::execution::Answer)).
pub(crate) enum PendingCommit {
    /// There is something for `git commit` to record — an open merge, or a
    /// dirty tree.
    Pending,
    /// git answered, and there is nothing left to record.
    Nothing,
    /// Neither probe reached a verdict. The payload is git's own words, for a
    /// caller that has to say why it stopped.
    Unreadable(String),
}

/// Is there anything for `git commit` to record in `wt_path` — either an
/// in-progress merge to conclude, or modified tracked files?
///
/// `git status --porcelain` is empty exactly when the tree is clean, and
/// `MERGE_HEAD` exists exactly while a merge is awaiting its commit. An
/// agent that resolved *and committed* leaves neither.
pub(crate) async fn probe(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    wt_path: &str,
) -> PendingCommit {
    let safe = paths::shell_escape_posix(wt_path);
    // `--quiet` is what makes the refusal meaningful: no MERGE_HEAD exits 1
    // rather than printing a diagnostic.
    match ask(
        exec,
        machine_str,
        &format!("git -C {} rev-parse --verify --quiet MERGE_HEAD", safe),
    )
    .await
    {
        Answer::Said(out) if !out.trim().is_empty() => return PendingCommit::Pending,
        Answer::Unreadable(e) => return PendingCommit::Unreadable(e),
        Answer::Said(_) | Answer::Refused => {}
    }
    match ask(
        exec,
        machine_str,
        &format!("git -C {} status --porcelain", safe),
    )
    .await
    {
        Answer::Said(out) if !out.trim().is_empty() => PendingCommit::Pending,
        Answer::Said(_) => PendingCommit::Nothing,
        // A worktree that refuses a porcelain read is not reporting a clean
        // tree, it is not reporting.
        Answer::Refused => PendingCommit::Unreadable(format!(
            "`git status` refused to read the worktree at {}",
            wt_path
        )),
        Answer::Unreadable(e) => PendingCommit::Unreadable(e),
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/steps/pending_commit.rs"]
mod tests;
