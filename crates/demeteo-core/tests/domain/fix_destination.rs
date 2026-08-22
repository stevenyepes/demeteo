//! Where a fix run's commits land, with no provider and no remote under it.
//! `super` is `crate::domain::fix_destination`.

use super::*;

use serde_json::Value;

const SAME_REPO: &str = include_str!("../fixtures/mr_summary/github-same-repo.json");
const FORK: &str = include_str!("../fixtures/mr_summary/github-fork.json");
const UNAUTHENTICATED: &str =
    include_str!("../fixtures/mr_summary/github-draft-unauthenticated.json");
const GITLAB_FORK: &str = include_str!("../fixtures/mr_summary/gitlab-fork.json");

fn payload(raw: &str) -> Value {
    serde_json::from_str(raw).expect("fixture is not JSON")
}

fn github(raw: &str) -> MrSummary {
    MrSummary::from_github(&payload(raw)).expect("fixture is not a pull request")
}

fn stacked_on(base: &str) -> FixDestination {
    FixDestination::StackedPr {
        base: base.to_string(),
    }
}

#[test]
fn a_same_repo_request_stacks_on_the_branch_under_review() {
    assert_eq!(
        resolve(&github(SAME_REPO)),
        stacked_on("feature/windows-shell"),
        "the head branch is upstream and the provider confirmed push on it, so the fix \
         can be reviewed against the work it fixes rather than against master"
    );
}

#[test]
fn a_fork_request_stacks_on_the_branch_the_review_targets() {
    assert_eq!(
        resolve(&github(FORK)),
        stacked_on("master"),
        "patch-1 lives in the fork; a pull request opened on origin cannot name it as a base"
    );

    let mut raw = payload(FORK);
    raw["head"]["repo"]["permissions"]["push"] = Value::Bool(true);
    let writable_fork = MrSummary::from_github(&raw).expect("fixture is not a pull request");
    assert_eq!(
        resolve(&writable_fork),
        stacked_on("master"),
        "push on the fork writes inside the fork; it does not put patch-1 upstream"
    );
}

#[test]
fn a_request_whose_permissions_the_provider_omitted_stacks_on_the_target() {
    let unauthenticated = github(UNAUTHENTICATED);
    assert!(
        !unauthenticated.from_fork,
        "the fixture is same-repo, so only the missing permissions decide this"
    );
    assert_eq!(
        resolve(&unauthenticated),
        stacked_on("master"),
        "a payload that never said we may add commits to wip/review-lane has not said yes"
    );
}

/// The degradation, asserted as a degradation. A same-project merge request's
/// head branch is upstream by construction, so the only thing keeping it off
/// the head branch is the permission GitLab never reports — see
/// `docs/KNOWN_ISSUES.md`, which is where a user looking for the missing
/// behaviour will go.
#[test]
fn no_merge_request_stacks_on_its_head_because_gitlab_reports_no_push_permission() {
    let mut raw = payload(GITLAB_FORK);
    raw["source_project_id"] = raw["target_project_id"].clone();
    let same_project = MrSummary::from_gitlab(&raw).expect("fixture is not a merge request");

    assert!(
        !same_project.from_fork,
        "the fixture is same-project, so only the missing permission decides this"
    );
    assert!(
        !same_project.head_repo_push,
        "this is the whole cause: a merge request carries no field for GitLab to report it in"
    );
    assert_eq!(resolve(&same_project), stacked_on("main"));
}

/// Split from the case above so it cannot pass for that case's reason. Handing
/// the fork the permission it never gets from GitLab leaves `from_fork` as the
/// only term still deciding — which is the term that answers the hard question
/// (is this branch upstream at all) rather than the permissive one.
#[test]
fn a_forked_merge_request_stacks_on_its_target_even_when_push_is_held() {
    let mut fork =
        MrSummary::from_gitlab(&payload(GITLAB_FORK)).expect("fixture is not a merge request");
    assert!(fork.from_fork, "the fixture is a fork");

    fork.head_repo_push = true;
    assert_eq!(
        resolve(&fork),
        stacked_on("main"),
        "the source branch lives in the fork; holding push on it does not put it upstream"
    );
}
