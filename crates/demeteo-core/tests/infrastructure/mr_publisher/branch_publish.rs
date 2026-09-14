//! `publish_branch_mr` is keyed on the branches the caller passes in, not on
//! any `Feature` — this proves the outgoing POST carries exactly those
//! branches (AC6), for both providers, without a `Feature` row to fall back
//! to. See `target_branch.rs` for the `Feature`-keyed counterpart this must
//! not regress.

use std::sync::Arc;

use rusqlite::Connection;

use super::{FakeHttpClient, HttpMrPublisher};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::ids::{ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, ProviderInstance, PublishOptions, Repository};
use crate::ports::db::{AppSettingsRepository, ProjectRepository};
use crate::ports::mr_publisher::MrPublisher;

const PROJECT: &str = "p-1";
const REPO_PATH: &str = "acme/widget";
const SOURCE_BRANCH: &str = "discovery/spike-1";
const TARGET_BRANCH: &str = "release/2.0";

const GITHUB_URL: &str = "https://api.github.com/repos/acme/widget/pulls";
const GITLAB_URL: &str = "https://gitlab.com/api/v4/projects/acme%2Fwidget/merge_requests";
const PROVIDER_ID: &str = "prov-1";

/// A project of one repository on one provider, with the provider's token in
/// the credential cache so `resolve_pat` answers without an OS keyring —
/// `publish_branch_mr` has no PAT-override variant, unlike `publish_mr_with_pat`.
fn seeded(kind: &str, host: &str) -> Arc<SqliteAdapter> {
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
            id: ProviderId::from(PROVIDER_ID),
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
            provider_id: ProviderId::from(PROVIDER_ID),
            repo_path: REPO_PATH.to_string(),
        },
    )
    .unwrap();
    crate::credential_cache::set(PROVIDER_ID, "not-a-real-token");
    adapter
}

/// Answers no `git` invocation — `publish_branch_mr` must never push, so a
/// call recorded here fails the test rather than reaching the assertion
/// having pushed something.
fn no_push_exec() -> Arc<ScriptedExec> {
    Arc::new(ScriptedExec::new(&[]))
}

fn options() -> PublishOptions {
    PublishOptions {
        draft: false,
        title: None,
        body: None,
        target_branch: None,
    }
}

fn github_http() -> Arc<FakeHttpClient> {
    Arc::new(FakeHttpClient::new().reply(
        GITHUB_URL,
        201,
        r#"{"html_url":"https://github.com/acme/widget/pull/9","number":9,"state":"open"}"#,
    ))
}

fn gitlab_http() -> Arc<FakeHttpClient> {
    Arc::new(FakeHttpClient::new().reply(
        GITLAB_URL,
        201,
        r#"{"web_url":"https://gitlab.com/acme/widget/-/merge_requests/9","iid":9,"state":"opened"}"#,
    ))
}

fn field(http: &FakeHttpClient, url: &str, key: &str) -> String {
    http.posted_to(url)
        .expect("the publisher POSTed a request")
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

#[tokio::test]
async fn a_github_pr_carries_exactly_the_branches_given() {
    let adapter = seeded("github", "github.com");
    let http = github_http();
    let publisher = HttpMrPublisher::with_http_override(
        adapter.clone(),
        adapter.clone(),
        adapter,
        no_push_exec(),
        http.clone(),
    );

    publisher
        .publish_branch_mr(PROJECT, SOURCE_BRANCH, TARGET_BRANCH, options())
        .await
        .expect("the provider answered a created request");

    assert_eq!(field(&http, GITHUB_URL, "head"), SOURCE_BRANCH);
    assert_eq!(field(&http, GITHUB_URL, "base"), TARGET_BRANCH);
}

#[tokio::test]
async fn a_gitlab_mr_carries_exactly_the_branches_given() {
    let adapter = seeded("gitlab", "gitlab.com");
    let http = gitlab_http();
    let publisher = HttpMrPublisher::with_http_override(
        adapter.clone(),
        adapter.clone(),
        adapter,
        no_push_exec(),
        http.clone(),
    );

    publisher
        .publish_branch_mr(PROJECT, SOURCE_BRANCH, TARGET_BRANCH, options())
        .await
        .expect("the provider answered a created request");

    assert_eq!(field(&http, GITLAB_URL, "source_branch"), SOURCE_BRANCH);
    assert_eq!(field(&http, GITLAB_URL, "target_branch"), TARGET_BRANCH);
}
