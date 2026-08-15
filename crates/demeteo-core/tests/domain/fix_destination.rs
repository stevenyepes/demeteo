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

#[test]
fn a_merge_request_stacks_on_its_target_whether_or_not_it_forks() {
    let fork =
        MrSummary::from_gitlab(&payload(GITLAB_FORK)).expect("fixture is not a merge request");
    assert_eq!(resolve(&fork), stacked_on("main"));

    let mut raw = payload(GITLAB_FORK);
    raw["source_project_id"] = raw["target_project_id"].clone();
    let same_project = MrSummary::from_gitlab(&raw).expect("fixture is not a merge request");
    assert_eq!(
        resolve(&same_project),
        stacked_on("main"),
        "GitLab reports no push permission for any merge request, so none of them stack"
    );
}
