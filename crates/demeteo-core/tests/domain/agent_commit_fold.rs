//! The fold decision and its report, over plain observations. No git.

use super::*;

const PRE: &str = "1111111111111111111111111111111111111111";
const POST: &str = "2222222222222222222222222222222222222222";
const BRANCH: &str = "refs/heads/feat_subtask_t1";

fn head(sha: Option<&str>, branch: Option<&str>) -> HeadState {
    HeadState {
        sha: sha.map(str::to_string),
        branch: branch.map(str::to_string),
    }
}

fn fold_to_pin() -> FoldPlan {
    FoldPlan::Fold {
        branch: BRANCH.to_string(),
        reset_to: PRE.to_string(),
    }
}

#[test]
fn an_unmoved_head_is_clean() {
    let at = head(Some(PRE), Some(BRANCH));
    assert_eq!(plan_fold(&at, &at), FoldPlan::Clean);
}

/// Both reads failing is not evidence the agent did anything.
#[test]
fn two_unreadable_heads_are_clean() {
    assert_eq!(
        plan_fold(&HeadState::default(), &HeadState::default()),
        FoldPlan::Clean
    );
}

#[test]
fn a_commit_on_the_branch_folds_back_to_the_pin() {
    assert_eq!(
        plan_fold(
            &head(Some(PRE), Some(BRANCH)),
            &head(Some(POST), Some(BRANCH))
        ),
        fold_to_pin()
    );
}

/// A branch switch with no new commit still moves HEAD off the subtask
/// branch, and Demeteo's commit would land on the wrong ref.
#[test]
fn a_branch_switch_alone_folds() {
    assert_eq!(
        plan_fold(
            &head(Some(PRE), Some(BRANCH)),
            &head(Some(PRE), Some("refs/heads/agent-scratch"))
        ),
        fold_to_pin()
    );
}

#[test]
fn a_detached_head_after_the_turn_folds() {
    assert_eq!(
        plan_fold(&head(Some(PRE), Some(BRANCH)), &head(Some(POST), None)),
        fold_to_pin()
    );
}

#[test]
fn without_a_pre_turn_sha_there_is_nothing_to_return_to() {
    assert!(matches!(
        plan_fold(&head(None, Some(BRANCH)), &head(Some(POST), Some(BRANCH))),
        FoldPlan::CannotFold { .. }
    ));
    assert!(matches!(
        plan_fold(
            &head(Some(""), Some(BRANCH)),
            &head(Some(POST), Some(BRANCH))
        ),
        FoldPlan::CannotFold { .. }
    ));
}

#[test]
fn a_turn_that_started_detached_cannot_name_a_branch() {
    assert!(matches!(
        plan_fold(&head(Some(PRE), None), &head(Some(POST), None)),
        FoldPlan::CannotFold { .. }
    ));
}

#[test]
fn an_unreadable_head_after_the_turn_is_not_folded() {
    assert!(matches!(
        plan_fold(&head(Some(PRE), Some(BRANCH)), &head(None, None)),
        FoldPlan::CannotFold { .. }
    ));
}

#[test]
fn log_lines_parse_into_commits() {
    let log = format!(
        "{POST}\u{1f}Jane Doe <jane@example.com>\u{1f}feat: add the thing\n\
         {PRE}\u{1f}Jane Doe <jane@example.com>\u{1f}wip: a | pipe \u{2014} and dash\n\n"
    );
    assert_eq!(
        parse_agent_commits(&log),
        vec![
            AgentCommit {
                sha: POST.to_string(),
                author: "Jane Doe <jane@example.com>".to_string(),
                subject: "feat: add the thing".to_string(),
            },
            AgentCommit {
                sha: PRE.to_string(),
                author: "Jane Doe <jane@example.com>".to_string(),
                subject: "wip: a | pipe \u{2014} and dash".to_string(),
            },
        ]
    );
}

#[test]
fn a_line_without_an_author_field_is_dropped() {
    assert!(parse_agent_commits("not a log line\n").is_empty());
}

#[test]
fn the_note_names_the_count_the_author_and_every_subject() {
    let commit = |subject: &str| AgentCommit {
        sha: POST.to_string(),
        author: "Jane <j@x>".to_string(),
        subject: subject.to_string(),
    };
    assert_eq!(
        fold_note(&[commit("one"), commit("two")]),
        "Agent made 2 commits as Jane <j@x> \u{2014} folded into Demeteo's commit: \"one\", \"two\""
    );
    assert_eq!(
        fold_note(&[commit("one")]),
        "Agent made 1 commit as Jane <j@x> \u{2014} folded into Demeteo's commit: \"one\""
    );
}

#[test]
fn a_fold_with_no_listed_commits_still_says_what_happened() {
    assert!(fold_note(&[]).contains("folded into Demeteo's commit"));
}
