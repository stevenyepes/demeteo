// `super` = `adapters::step_executor::sync_worktree`.
//
// The guard is the point: an `rm -rf` over the main repo checkout is what it
// prevents, and it used to be spelled by each caller rather than by the
// function. Everything here goes through a double that errors on any command
// it was not told to answer, so "it issued something else" is a failure.

use super::*;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo-sync";

fn scripted(answers: &[(&str, Result<&str, &str>)]) -> ScriptedExec {
    ScriptedExec::new(answers)
}

const REMOVE: &str = "git -C /repos/demeteo worktree remove --force /repos/demeteo-sync";
const RM: &str = "rm -rf /repos/demeteo-sync";
const PRUNE: &str = "git -C /repos/demeteo worktree prune";

#[tokio::test]
async fn the_three_commands_issue_in_order_against_the_right_paths() {
    let exec = scripted(&[(REMOVE, Ok("")), (RM, Ok("")), (PRUNE, Ok(""))]);
    discard_sync_worktree(&exec, "local", REPO, WT).await;
    assert_eq!(
        exec.commands(),
        vec![REMOVE.to_string(), RM.to_string(), PRUNE.to_string()]
    );
}

#[tokio::test]
async fn a_worktree_that_is_the_repo_itself_is_never_touched() {
    // The guard. Without it this is `rm -rf` over the user's checkout.
    let exec = scripted(&[]);
    discard_sync_worktree(&exec, "local", REPO, REPO).await;
    assert!(exec.commands().is_empty(), "issued {:?}", exec.commands());
}

#[tokio::test]
async fn a_failing_remove_still_lets_the_delete_and_the_prune_run() {
    // All three are `let _ =` today: a worktree git refuses to unregister
    // must still be deleted, and the stale entry must still be pruned.
    let exec = scripted(&[
        (REMOVE, Err("fatal: is not a working tree")),
        (RM, Ok("")),
        (PRUNE, Ok("")),
    ]);
    discard_sync_worktree(&exec, "local", REPO, WT).await;
    assert_eq!(
        exec.commands(),
        vec![REMOVE.to_string(), RM.to_string(), PRUNE.to_string()]
    );
}
