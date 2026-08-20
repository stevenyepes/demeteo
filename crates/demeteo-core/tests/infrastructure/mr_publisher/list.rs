//! The listing adapter, driven over the one port it reads.
//!
//! Every case here exists to hold one line: a provider status outside 2xx must
//! never become an `Ok`. The sibling `fetch_mr_state` path does exactly that
//! (`if resp.status >= 300 { return Ok("open") }`), it sits two screens away in
//! the same files, and copying it into the list path is a one-key mistake that
//! nothing else in the tree would catch — the listing would simply render as an
//! empty queue and tell the user nothing needs review.

use super::{github, gitlab, FakeHttpClient, ListRequest, MrListError};

const GITHUB_URL: &str = "https://api.github.com/repos/acme/widget/pulls?state=open&sort=updated&direction=desc&per_page=100";
const GITLAB_URL: &str = "https://gitlab.com/api/v4/projects/acme%2Fwidget/merge_requests?state=opened&order_by=updated_at&sort=desc&per_page=100";

fn github_request() -> ListRequest<'static> {
    ListRequest {
        kind: "github",
        host: "github.com",
        repo_path: "acme/widget",
        pat: "token",
    }
}

fn gitlab_request() -> ListRequest<'static> {
    ListRequest {
        kind: "gitlab",
        host: "gitlab.com",
        repo_path: "acme/widget",
        pat: "token",
    }
}

async fn github_list(http: FakeHttpClient) -> Result<Vec<serde_json::Value>, MrListError> {
    github::list_github_pulls(&http, &github_request()).await
}

async fn gitlab_list(http: FakeHttpClient) -> Result<Vec<serde_json::Value>, MrListError> {
    gitlab::list_gitlab_merge_requests(&http, &gitlab_request()).await
}

#[tokio::test]
async fn a_200_yields_the_elements() {
    let body = r#"[{"number":1},{"number":2}]"#;
    let items = github_list(FakeHttpClient::new().reply(GITHUB_URL, 200, body))
        .await
        .expect("a 200 lists");
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn a_401_is_unauthorized_and_never_an_empty_list() {
    let err = github_list(FakeHttpClient::new().reply(
        GITHUB_URL,
        401,
        r#"{"message":"Bad credentials"}"#,
    ))
    .await
    .expect_err("a 401 must not list");
    assert_eq!(
        err,
        MrListError::Unauthorized {
            provider: "github".into(),
            host: "github.com".into(),
            status: 401,
        }
    );
}

#[tokio::test]
async fn a_429_carries_its_retry_after() {
    let err = github_list(FakeHttpClient::new().reply_with_headers(
        GITHUB_URL,
        429,
        "slow down",
        &[("Retry-After", "60")],
    ))
    .await
    .expect_err("a 429 must not list");
    assert_eq!(
        err,
        MrListError::RateLimited {
            host: "github.com".into(),
            retry_after: Some(60),
        }
    );
}

#[tokio::test]
async fn a_500_keeps_the_providers_words() {
    let err = github_list(FakeHttpClient::new().reply(GITHUB_URL, 500, "upstream exploded"))
        .await
        .expect_err("a 500 must not list");
    assert_eq!(
        err,
        MrListError::Http {
            host: "github.com".into(),
            status: Some(500),
            body: "upstream exploded".into(),
        }
    );
}

#[tokio::test]
async fn no_failing_status_coerces_to_ok() {
    for status in [301, 400, 401, 403, 404, 422, 429, 500, 502, 503] {
        let result = github_list(FakeHttpClient::new().reply(GITHUB_URL, status, "[]")).await;
        assert!(
            result.is_err(),
            "HTTP {status} answered with an empty array must still fail — an empty \
             queue reads as 'nothing needs review'"
        );
    }
}

#[tokio::test]
async fn a_body_that_is_not_an_array_fails_rather_than_listing_nothing() {
    let err = github_list(FakeHttpClient::new().reply(GITHUB_URL, 200, r#"{"message":"nope"}"#))
        .await
        .expect_err("an object is not a list");
    assert!(matches!(err, MrListError::Http { status: None, .. }));
}

#[tokio::test]
async fn a_transport_failure_is_reported_not_swallowed() {
    // The fake answers nothing, which is how a DNS or TLS failure arrives.
    let err = github_list(FakeHttpClient::new())
        .await
        .expect_err("an unanswered request must not list");
    assert!(matches!(err, MrListError::Http { status: None, .. }));
}

#[tokio::test]
async fn gitlab_reads_its_own_endpoint_and_fails_the_same_way() {
    let items = gitlab_list(FakeHttpClient::new().reply(GITLAB_URL, 200, r#"[{"iid":9}]"#))
        .await
        .expect("a 200 lists");
    assert_eq!(items.len(), 1);

    let err = gitlab_list(FakeHttpClient::new().reply(GITLAB_URL, 401, "unauthorized"))
        .await
        .expect_err("a 401 must not list");
    assert_eq!(
        err,
        MrListError::Unauthorized {
            provider: "gitlab".into(),
            host: "gitlab.com".into(),
            status: 401,
        }
    );
}
