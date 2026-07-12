// Tests for `steps/finalize/prompt.rs` (mirrored-tests convention).

use super::*;
use crate::adapters::step_executor::steps::finalize::context::BranchWork;

fn work() -> BranchWork {
    BranchWork {
        commit_log: "- step 1 work".to_string(),
        diff_stat: " src/a.rs | 2 +-".to_string(),
        diff: "--- a/src/a.rs\n+++ b/src/a.rs".to_string(),
        diff_truncated: false,
        conventions: "--- commitlint.config.js ---\nfeat|fix|chore".to_string(),
    }
}

/// The prompt has to *tell* the agent that publishing isn't its job — the
/// tool policy already makes it impossible, but an agent that doesn't know
/// that burns a turn discovering it.
#[test]
fn the_authoring_prompt_states_that_demeteo_opens_the_pr() {
    let p = build_authoring_prompt("Add retries", "desc", "feature/f-1", "main", &work());
    assert!(p.contains("no shell and no network"));
    assert!(p.contains("Demeteo squashes the branch and opens the pull request itself"));
    assert!(
        p.contains("gh"),
        "the prompt should name the tools it must not reach for"
    );
}

#[test]
fn the_authoring_prompt_carries_the_work_and_the_repo_conventions() {
    let p = build_authoring_prompt(
        "Add retries",
        "the description",
        "feature/f-1",
        "main",
        &work(),
    );
    assert!(p.contains("the description"));
    assert!(p.contains("- step 1 work"));
    assert!(p.contains("src/a.rs"));
    assert!(p.contains("commitlint.config.js"));
    assert!(p.contains("feature/f-1"));
    assert!(p.contains("main"));
    // All four keys of the wire contract.
    for key in ["commit_subject", "commit_body", "pr_title", "pr_body"] {
        assert!(p.contains(key), "prompt must specify the {key} field");
    }
}

#[test]
fn the_authoring_prompt_flags_a_truncated_diff() {
    let mut w = work();
    w.diff_truncated = true;
    let p = build_authoring_prompt("t", "d", "feature/f-1", "main", &w);
    assert!(p.contains("truncated"));
}

/// The repair prompt is what turns a commitlint rejection from a wedged
/// pipeline into another turn of a loop that converges.
#[test]
fn the_repair_prompt_hands_the_hook_verdict_back_to_the_agent() {
    let p = build_repair_prompt(
        "Merge subtask sub-2",
        "✖ subject may not be empty [subject-empty]\n✖ type may not be empty [type-empty]",
    );
    assert!(p.contains("REJECTED"));
    assert!(
        p.contains("Merge subtask sub-2"),
        "it must see what it proposed"
    );
    assert!(
        p.contains("subject-empty"),
        "it must see the hook's own complaint"
    );
    assert!(p.contains("pr_title"), "the wire contract is restated");
}
