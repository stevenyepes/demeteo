//! The single-request read, driven over the one port it uses.
//!
//! It shares its URL with the state poll and nothing else. The poll coerces
//! every non-2xx to `"open"` on purpose, which is right for a badge nobody acts
//! on and wrong here: this answer becomes a mergeability verdict on a row, so a
//! rejected request has to arrive as a rejection.

use super::{github, gitlab, FakeHttpClient, MrListError};
use crate::domain::mr_summary::MrSummary;

const GITHUB_URL: &str = "https://api.github.com/repos/acme/widget/pulls/7";
const GITLAB_URL: &str = "https://gitlab.com/api/v4/projects/acme%2Fwidget/merge_requests/7";
const GITHUB_PR: &str = "https://github.com/acme/widget/pull/7";
const GITLAB_MR: &str = "https://gitlab.com/acme/widget/-/merge_requests/7";

#[tokio::test]
async fn a_200_carries_the_verdict_the_listing_could_not() {
    let body = r#"{
        "number": 7,
        "html_url": "https://github.com/acme/widget/pull/7",
        "head": { "ref": "topic", "repo": { "full_name": "acme/widget" } },
        "base": { "ref": "main", "repo": { "full_name": "acme/widget" } },
        "mergeable": false,
        "additions": 4,
        "deletions": 2,
        "changed_files": 1
    }"#;
    let payload = github::fetch_github_pr_detail(
        &FakeHttpClient::new().reply(GITHUB_URL, 200, body),
        "github.com",
        GITHUB_PR,
        "token",
    )
    .await
    .expect("a 200 reads");

    let pr = MrSummary::from_github(&payload).expect("the detail payload is a pull request");
    assert_eq!(pr.has_conflicts, Some(true));
    assert_eq!(pr.changed_files, Some(1));
}

#[tokio::test]
async fn a_throttled_403_is_a_rate_limit_and_never_a_clean_merge() {
    let err = github::fetch_github_pr_detail(
        &FakeHttpClient::new().reply_with_headers(
            GITHUB_URL,
            403,
            r#"{"message":"API rate limit exceeded"}"#,
            &[("x-ratelimit-remaining", "0")],
        ),
        "github.com",
        GITHUB_PR,
        "token",
    )
    .await
    .expect_err("a 403 must not read as a request");

    assert_eq!(
        err,
        MrListError::RateLimited {
            host: "github.com".into(),
            retry_after: None,
        }
    );
}

#[tokio::test]
async fn a_404_is_reported_rather_than_coerced_to_open() {
    let err = gitlab::fetch_gitlab_mr_detail(
        &FakeHttpClient::new().reply(GITLAB_URL, 404, r#"{"message":"404 Not found"}"#),
        "gitlab.com",
        GITLAB_MR,
        "token",
    )
    .await
    .expect_err("a 404 must not read as a merge request");

    assert!(
        matches!(
            err,
            MrListError::Http {
                status: Some(404),
                ..
            }
        ),
        "got {err:?}"
    );
}
