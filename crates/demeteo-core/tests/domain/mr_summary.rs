//! Provider payloads mapped to one summary, with no provider under them.
//! `super` is `crate::domain::mr_summary`.

use super::*;

use crate::domain::feature_origin::FeatureOrigin;

const SAME_REPO: &str = include_str!("../fixtures/mr_summary/github-same-repo.json");
const FORK: &str = include_str!("../fixtures/mr_summary/github-fork.json");
const DRAFT: &str = include_str!("../fixtures/mr_summary/github-draft-unauthenticated.json");
const GITLAB_FORK: &str = include_str!("../fixtures/mr_summary/gitlab-fork.json");
const GITHUB_CHECKING: &str = include_str!("../fixtures/mr_summary/github-detail-checking.json");
const GITHUB_CONFLICTING: &str =
    include_str!("../fixtures/mr_summary/github-detail-conflicting.json");
const GITLAB_CHECKING: &str = include_str!("../fixtures/mr_summary/gitlab-detail-checking.json");
const GITLAB_CONFLICTING: &str =
    include_str!("../fixtures/mr_summary/gitlab-detail-conflicting.json");

fn payload(raw: &str) -> Value {
    serde_json::from_str(raw).expect("fixture is not JSON")
}

fn github(raw: &str) -> MrSummary {
    MrSummary::from_github(&payload(raw)).expect("fixture is not a pull request")
}

fn gitlab(raw: &str) -> MrSummary {
    MrSummary::from_gitlab(&payload(raw)).expect("fixture is not a merge request")
}

#[test]
fn a_same_repo_pull_request_maps_whole() {
    let pr = github(SAME_REPO);

    assert_eq!(pr.number, 118);
    assert_eq!(
        pr.title,
        "declare each harness's Windows shell instead of its kind"
    );
    assert_eq!(pr.author, "stevenyepes");
    assert_eq!(pr.source_branch, "feature/windows-shell");
    assert_eq!(pr.target_branch, "master");
    assert!(!pr.draft);
    assert_eq!(pr.web_url, "https://github.com/stvcloud/demeteo/pull/118");
    assert_eq!(pr.created_at, "2026-08-01T09:12:44Z");
    assert_eq!(pr.updated_at, "2026-08-02T17:03:10Z");
    assert_eq!(pr.head_repo_path.as_deref(), Some("stvcloud/demeteo"));
    assert_eq!(pr.head_fetch_spec.as_str(), "refs/pull/118/head");
    assert!(!pr.from_fork);
    assert!(pr.maintainer_can_modify);
    assert!(pr.head_repo_push);
}

#[test]
fn a_fork_pull_request_is_still_fetched_from_the_upstream_repository() {
    let pr = github(FORK);

    assert!(pr.from_fork);
    assert_eq!(
        pr.head_repo_path.as_deref(),
        Some("outside-contributor/demeteo")
    );
    assert_eq!(
        pr.head_fetch_spec.as_str(),
        "refs/pull/204/head",
        "patch-1 lives in the fork; only the upstream pull ref is reachable from this clone"
    );
    assert!(pr.maintainer_can_modify);
    assert!(
        !pr.head_repo_push,
        "the payload says push is denied on the fork, and it is the payload that decides"
    );
}

#[test]
fn a_head_repository_the_payload_does_not_name_reads_as_a_fork() {
    let mut raw = payload(SAME_REPO);
    raw["head"]["repo"] = Value::Null;
    let pr = MrSummary::from_github(&raw).expect("a deleted head repo is still a pull request");

    assert_eq!(pr.head_repo_path, None);
    assert!(
        pr.from_fork,
        "an unnamed head repository is not this repository"
    );
    assert!(!pr.head_repo_push);
}

#[test]
fn a_draft_stays_a_draft() {
    assert!(github(DRAFT).draft);
    assert!(!github(SAME_REPO).draft);
}

#[test]
fn a_permission_the_payload_omits_resolves_to_not_permitted() {
    let pr = github(DRAFT);
    assert!(
        !pr.maintainer_can_modify,
        "an omitted maintainer_can_modify says we do not know, and we do not know is not yes"
    );
    assert!(
        !pr.head_repo_push,
        "an omitted permissions block says the same about push"
    );

    let mut raw = payload(GITLAB_FORK);
    raw.as_object_mut()
        .expect("the merge request fixture is an object")
        .remove("allow_collaboration");
    let mr = MrSummary::from_gitlab(&raw).expect("allow_collaboration is not identity");
    assert!(!mr.maintainer_can_modify);
}

#[test]
fn a_merge_request_is_fetched_by_its_merge_request_ref() {
    let mr = gitlab(GITLAB_FORK);

    assert_eq!(mr.number, 42);
    assert_eq!(mr.title, "carry a run's origin to the remote runner");
    assert_eq!(mr.author, "outside-contributor");
    assert_eq!(mr.source_branch, "feature/runner-origin");
    assert_eq!(mr.target_branch, "main");
    assert!(!mr.draft);
    assert_eq!(
        mr.web_url,
        "https://gitlab.com/stvcloud/demeteo/-/merge_requests/42"
    );
    assert_eq!(mr.head_fetch_spec.as_str(), "refs/merge-requests/42/head");
    assert!(mr.from_fork, "the source project is not the target project");
    assert_eq!(
        mr.head_repo_path, None,
        "a merge request names its source project by id only"
    );
    assert!(mr.maintainer_can_modify);
    assert!(
        !mr.head_repo_push,
        "a merge request carries no push permission to report"
    );
}

#[test]
fn a_merge_request_within_one_project_is_not_a_fork() {
    let mut raw = payload(GITLAB_FORK);
    raw["source_project_id"] = raw["target_project_id"].clone();
    let mr = MrSummary::from_gitlab(&raw).expect("fixture is not a merge request");

    assert!(!mr.from_fork);
}

#[test]
fn a_payload_naming_no_head_is_not_a_summary() {
    let mut raw = payload(SAME_REPO);
    raw.as_object_mut()
        .expect("the pull request fixture is an object")
        .remove("head");

    assert!(
        MrSummary::from_github(&raw).is_err(),
        "a request with no head branch describes nothing to review"
    );
}

#[test]
fn the_head_fetch_spec_is_one_a_run_can_start_from() {
    for raw in [SAME_REPO, FORK] {
        let pr = github(raw);
        let origin = FeatureOrigin::Ref {
            fetch_spec: pr.head_fetch_spec.as_str().to_string(),
            label: pr.title.clone(),
        };
        assert!(
            origin.fetch_plan("master").is_ok(),
            "a summary must not hold a spec the bootstrap refuses"
        );
    }
}

#[test]
fn a_mergeability_still_being_computed_is_not_a_clean_merge() {
    let checking = github(GITHUB_CHECKING);
    assert_eq!(
        checking.has_conflicts, None,
        "GitHub answers `mergeable: null` until it has finished deciding; \
         reading that as a clean merge hands a reviewer a verdict nobody gave"
    );

    let conflicting = github(GITHUB_CONFLICTING);
    assert_eq!(conflicting.has_conflicts, Some(true));

    let checking = gitlab(GITLAB_CHECKING);
    assert_eq!(
        checking.has_conflicts, None,
        "`merge_status: checking` is the same undecided answer in GitLab's spelling"
    );

    let conflicting = gitlab(GITLAB_CONFLICTING);
    assert_eq!(conflicting.has_conflicts, Some(true));
}

#[test]
fn the_review_tier_is_read_where_a_payload_carries_it() {
    let pr = github(GITHUB_CHECKING);
    assert_eq!(pr.additions, Some(120));
    assert_eq!(pr.deletions, Some(8));
    assert_eq!(pr.changed_files, Some(3));

    let mr = gitlab(GITLAB_CHECKING);
    assert_eq!(
        mr.changed_files,
        Some(1000),
        "GitLab caps `changes_count` at \"1000+\"; the floor is what the label means, \
         and a failed parse would claim the request touches nothing"
    );
    assert_eq!(
        (mr.additions, mr.deletions),
        (None, None),
        "neither GitLab merge-request endpoint carries a line diffstat"
    );
}

/// A field the review tier added must not become a field a listing requires.
///
/// `list_one_repo` drops an element it cannot map, so one missing
/// `#[serde(default)]` empties the whole queue and reports nothing wrong. The
/// list fixtures carry none of the four, which is exactly the payload that
/// would trip it.
#[test]
fn a_list_element_missing_the_review_tier_still_maps() {
    for (label, pr) in [
        ("github", github(SAME_REPO)),
        ("github fork", github(FORK)),
        ("gitlab", gitlab(GITLAB_FORK)),
    ] {
        assert_eq!(
            (
                pr.has_conflicts,
                pr.additions,
                pr.deletions,
                pr.changed_files
            ),
            (None, None, None, None),
            "{label} list element must map with an empty review tier, not fail"
        );
    }
}

/// The undecided verdict has to survive the wire, or the frontend cannot tell
/// it from a clean one. The diffstat may be skipped; this may not.
#[test]
fn an_undecided_verdict_is_serialized_rather_than_skipped() {
    let json = serde_json::to_value(github(SAME_REPO)).expect("summary is serializable");
    assert_eq!(
        json.get("has_conflicts"),
        Some(&Value::Null),
        "an absent `has_conflicts` reads as a clean merge on the other side"
    );
    assert!(
        json.get("additions").is_none(),
        "absent and unknown are the same fact for a diffstat"
    );
}
