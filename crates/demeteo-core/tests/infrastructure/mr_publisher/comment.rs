//! Where a review report lands, driven over the one port the publisher reads.
//!
//! The endpoint is the whole test. GitHub's PR-level conversation is the
//! *issue*'s comment collection, and `POST /pulls/{n}/comments` — one word away,
//! and the name anyone reaches for — is a diff-line review comment that answers
//! 404 for want of a commit and a path. A 404 from a provider is
//! indistinguishable from a pull request that was closed or renamed, so nothing
//! downstream could tell the two apart; `FakeHttpClient` erroring on a URL it
//! was never told about is what makes the wrong one fail here instead.
//!
//! These drive [`MrPublisher::post_mr_comment`] rather than the two adapter
//! functions under it, because the attribution is applied between them and a
//! comment reaching a pull request unattributed is the failure the line exists
//! to prevent.

use std::sync::Arc;

use rusqlite::Connection;

use super::{FakeHttpClient, HttpMrPublisher};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::ids::{ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, ProviderInstance, Repository};
use crate::domain::mr_comment::ATTRIBUTION;
use crate::ports::db::{AppSettingsRepository, ProjectRepository};
use crate::ports::mr_publisher::MrPublisher;

const PROJECT: &str = "p-comment";
const REPO_PATH: &str = "acme/widget";
const REPORT: &str = "## Findings\n\nThe refspec guard holds.";

const PR_URL: &str = "https://github.com/acme/widget/pull/412";
const ISSUES_ENDPOINT: &str = "https://api.github.com/repos/acme/widget/issues/412/comments";
const COMMENT_URL: &str = "https://github.com/acme/widget/pull/412#issuecomment-1";

const MR_URL: &str = "https://gitlab.com/acme/widget/-/merge_requests/9";
const NOTES_ENDPOINT: &str =
    "https://gitlab.com/api/v4/projects/acme%2Fwidget/merge_requests/9/notes";

/// A project of one repository on one provider, with the provider's token in
/// the credential cache so `resolve_pat` answers without an OS keyring.
fn seeded(provider_id: &str, kind: &str, host: &str) -> Arc<SqliteAdapter> {
    let adapter = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    let pid = ProjectId::from(PROJECT.to_string());

    ProjectRepository::add(
        adapter.as_ref(),
        Project {
            id: pid.clone(),
            name: "widget".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    )
    .unwrap();
    adapter
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from(provider_id),
            kind: kind.to_string(),
            host: host.to_string(),
            username: "someone".to_string(),
            avatar_url: String::new(),
            created_at: 1000,
        })
        .unwrap();
    ProjectRepository::add_repository(
        adapter.as_ref(),
        Repository {
            id: RepositoryId::from("r-1"),
            project_id: pid,
            provider_id: ProviderId::from(provider_id),
            repo_path: REPO_PATH.to_string(),
        },
    )
    .unwrap();
    crate::credential_cache::set(provider_id, "not-a-real-token");
    adapter
}

fn publisher(adapter: Arc<SqliteAdapter>, http: Arc<FakeHttpClient>) -> HttpMrPublisher {
    HttpMrPublisher::with_http_override(
        adapter.clone(),
        adapter.clone(),
        adapter,
        Arc::new(ScriptedExec::new(&[])),
        http,
    )
}

fn posted_body(http: &FakeHttpClient, endpoint: &str) -> String {
    http.posted_to(endpoint)
        .expect("the publisher POSTed a comment")
        .get("body")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_github_report_goes_to_the_issue_comment_collection() {
    let http = Arc::new(FakeHttpClient::new().reply(
        ISSUES_ENDPOINT,
        201,
        &format!(r#"{{"html_url":"{COMMENT_URL}"}}"#),
    ));

    let url = publisher(seeded("prov-gh", "github", "github.com"), http)
        .post_mr_comment(PROJECT, PR_URL, REPORT)
        .await
        .expect("the provider created the comment");

    assert_eq!(url, COMMENT_URL);
}

#[tokio::test]
async fn the_posted_body_carries_the_report_and_says_what_wrote_it() {
    let http = Arc::new(FakeHttpClient::new().reply(
        ISSUES_ENDPOINT,
        201,
        &format!(r#"{{"html_url":"{COMMENT_URL}"}}"#),
    ));

    publisher(seeded("prov-gh", "github", "github.com"), http.clone())
        .post_mr_comment(PROJECT, PR_URL, REPORT)
        .await
        .expect("the provider created the comment");

    let body = posted_body(&http, ISSUES_ENDPOINT);
    assert!(body.starts_with(REPORT), "the report reached the provider");
    assert!(
        body.ends_with(ATTRIBUTION),
        "a reader can tell an agent wrote it: {body}"
    );
}

/// A token without `pull_requests: write` answers 403 with a body, and the
/// button's next state is decided by this `Result` alone — an `Ok` here would
/// report a comment that does not exist, complete with a URL nobody can open.
#[tokio::test]
async fn a_403_is_an_error_rather_than_a_comment_nobody_posted() {
    let http = Arc::new(FakeHttpClient::new().reply(
        ISSUES_ENDPOINT,
        403,
        r#"{"message":"Resource not accessible by personal access token"}"#,
    ));

    let err = publisher(seeded("prov-gh", "github", "github.com"), http)
        .post_mr_comment(PROJECT, PR_URL, REPORT)
        .await
        .expect_err("a rejected write must not read as a posted comment");

    assert!(err.contains("403"), "{err}");
    assert!(err.contains("not accessible"), "{err}");
}

#[tokio::test]
async fn an_empty_report_never_reaches_the_provider() {
    let err = publisher(
        seeded("prov-gh", "github", "github.com"),
        Arc::new(FakeHttpClient::new()),
    )
    .post_mr_comment(PROJECT, PR_URL, "   \n")
    .await
    .expect_err("there is nothing to post");

    assert!(err.contains("empty"), "{err}");
}

#[tokio::test]
async fn a_gitlab_note_answers_with_the_anchor_its_own_ui_links_to() {
    let http = Arc::new(FakeHttpClient::new().reply(NOTES_ENDPOINT, 201, r#"{"id":88}"#));

    let url = publisher(seeded("prov-gl", "gitlab", "gitlab.com"), http.clone())
        .post_mr_comment(PROJECT, MR_URL, REPORT)
        .await
        .expect("the provider created the note");

    assert_eq!(url, format!("{MR_URL}#note_88"));
    assert!(posted_body(&http, NOTES_ENDPOINT).ends_with(ATTRIBUTION));
}
