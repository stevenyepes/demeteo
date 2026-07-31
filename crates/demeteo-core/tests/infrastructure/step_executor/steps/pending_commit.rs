// `super` = `steps::conflict_pass`.
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

const WT: &str = "/wt/subtask";
const MERGE_HEAD: &str = "git -C /wt/subtask rev-parse --verify --quiet MERGE_HEAD";
const STATUS: &str = "git -C /wt/subtask status --porcelain";

#[tokio::test]
async fn a_merge_awaiting_its_commit_is_pending_whatever_the_tree_says() {
    // `MERGE_HEAD` short-circuits: `status` is never consulted, which the
    // strict double proves by erroring if it were.
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("a1b2c3d\n"))]);
    assert!(worktree_has_pending_commit(&exec, "local", WT).await);
    assert_eq!(exec.commands(), vec![MERGE_HEAD.to_string()]);
}

#[tokio::test]
async fn a_dirty_tree_with_no_merge_in_progress_is_pending() {
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("\n")), (STATUS, Ok(" M src/lib.rs\n"))]);
    assert!(worktree_has_pending_commit(&exec, "local", WT).await);
}

#[tokio::test]
async fn a_clean_tree_with_no_merge_in_progress_is_not_pending() {
    // The case an unconditional `git commit -am` would break: the agent
    // already resolved *and* committed, so there is nothing left to record.
    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok("  \n")), (STATUS, Ok("\n"))]);
    assert!(!worktree_has_pending_commit(&exec, "local", WT).await);
}

#[tokio::test]
async fn an_unanswerable_probe_reads_as_not_pending() {
    // Both probes are `.unwrap_or(false)` today. That is a deliberate
    // reading, not an oversight: it is out of scope to "fix" here.
    let exec = ScriptedExec::new(&[]);
    assert!(!worktree_has_pending_commit(&exec, "local", WT).await);

    let exec = ScriptedExec::new(&[(MERGE_HEAD, Ok(""))]);
    assert!(!worktree_has_pending_commit(&exec, "local", WT).await);
}
