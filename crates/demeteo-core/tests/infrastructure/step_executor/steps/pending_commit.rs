// `super` = `steps::pending_commit`.
//
// The rule this pins has never been asserted: an agent told to fix conflict
// markers very often stages and commits on its own, which consumes
// `MERGE_HEAD` and leaves a clean tree — so an unconditional `git commit -am`
// would exit non-zero and roll back a merge that in fact succeeded.
//
// The double errors on anything it was not told to answer, so "it asked git
// something else" is a failure rather than a default.

use super::*;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::ports::execution::TRANSPORT_ERROR_PREFIX;

const WT: &str = "/wt/subtask";
const MERGE_HEAD: &str = "git -C /wt/subtask rev-parse --verify --quiet MERGE_HEAD";
const STATUS: &str = "git -C /wt/subtask status --porcelain";

fn transport_dead() -> String {
    format!("{}Connection appears dead", TRANSPORT_ERROR_PREFIX)
}

#[tokio::test]
async fn a_merge_awaiting_its_commit_is_pending_whatever_the_tree_says() {
    // `MERGE_HEAD` short-circuits: `status` is never consulted, which the
    // strict double proves by erroring if it were.
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("a1b2c3d\n"))]);
    assert!(matches!(
        probe(&exec, "local", WT).await,
        PendingCommit::Pending
    ));
    assert_eq!(exec.commands(), vec![MERGE_HEAD.to_string()]);
}

#[tokio::test]
async fn a_dirty_tree_with_no_merge_in_progress_is_pending() {
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("\n")), (STATUS, Ok(" M src/lib.rs\n"))]);
    assert!(matches!(
        probe(&exec, "local", WT).await,
        PendingCommit::Pending
    ));
}

#[tokio::test]
async fn a_clean_tree_with_no_merge_in_progress_is_not_pending() {
    // The case an unconditional `git commit -am` would break: the agent
    // already resolved *and* committed, so there is nothing left to record.
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("  \n")), (STATUS, Ok("\n"))]);
    assert!(matches!(
        probe(&exec, "local", WT).await,
        PendingCommit::Nothing
    ));
}

/// The distinction the whole enum exists for. A dead channel on either probe
/// used to read as "nothing to commit", and the sync resolver acts on that by
/// skipping the commit, pushing a no-op, filing the session `Resolved` and
/// force-removing the worktree the agent's work is in.
#[tokio::test]
async fn a_dead_channel_is_not_a_clean_tree() {
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Err(&transport_dead()))]);
    assert!(matches!(
        probe(&exec, "local", WT).await,
        PendingCommit::Unreadable(_)
    ));

    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("")), (STATUS, Err(&transport_dead()))]);
    assert!(matches!(
        probe(&exec, "local", WT).await,
        PendingCommit::Unreadable(_)
    ));
}

/// A worktree that answers neither read is not reporting a clean tree either.
/// `git -C` on a directory that is gone exits non-zero, which is a verdict
/// about the *repository*, not about what is left to commit.
#[tokio::test]
async fn a_worktree_that_refuses_the_porcelain_is_unreadable() {
    let exec = ScriptedExec::new(&[]);
    assert!(matches!(
        probe(&exec, "local", WT).await,
        PendingCommit::Unreadable(_)
    ));
}
