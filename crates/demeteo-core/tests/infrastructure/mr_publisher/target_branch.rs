//! Which branch the request Demeteo POSTs asks to merge into.
//!
//! Asserted on the outgoing payload rather than on any resolver, because the
//! defect this covers was a resolved value nothing read: `PublishOptions`
//! carried a target branch, `publish_mr` threaded it through, and the payload
//! was built from the project default regardless. Every assertion here that
//! named the default branch would have passed against that code — so none does.

use std::sync::Arc;

use rusqlite::Connection;

use super::{FakeHttpClient, HttpMrPublisher};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::mr_publisher::push::push_request;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Feature, Project, ProviderInstance, PublishOptions, Repository};
use crate::ports::db::{AppSettingsRepository, FeatureRepository, ProjectRepository};
use crate::ports::execution::ProgramRequest;
use crate::ports::mr_publisher::MrPublisher;

const PROJECT: &str = "p-1";
const REPO_PATH: &str = "acme/widget";
const FEATURE: &str = "f-1";
const PAT: &str = "not-a-real-token";
const SOURCE_BRANCH: &str = "demeteo/features/f-1";

const GITHUB_URL: &str = "https://api.github.com/repos/acme/widget/pulls";
const GITLAB_URL: &str = "https://gitlab.com/api/v4/projects/acme%2Fwidget/merge_requests";

/// The project has no settings row, so the publisher falls back to
/// `fetch_default_settings` — whose `default_branch` is `main` and whose
/// `branch_prefix` yields [`SOURCE_BRANCH`].
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
            id: ProviderId::from("prov-1"),
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
            provider_id: ProviderId::from("prov-1"),
            repo_path: REPO_PATH.to_string(),
        },
    )
    .unwrap();
    adapter
}

fn add_feature(adapter: &SqliteAdapter, origin: FeatureOrigin) {
    let mut feature: Feature = serde_json::from_value(serde_json::json!({
        "id": FEATURE,
        "project_id": PROJECT,
        "title": "Add the widget",
        "status": "running",
        "total_cost": 0.0,
        "duration": "0s",
        "created_at": 1000,
        "mr_url": "",
        "mr_state": "",
    }))
    .expect("the seed names every field Feature requires");
    feature.origin = origin;
    FeatureRepository::add(adapter, feature).unwrap();
}

fn rendered(request: &ProgramRequest) -> String {
    std::iter::once(request.executable.as_str())
        .chain(request.args.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Answers the two `git` invocations the push makes and errors on everything
/// else, so a publisher that pushed something other than the run's branch
/// fails here rather than reaching the assertion with a default.
fn push_exec(remote_user: &str, host: &str) -> Arc<ScriptedExec> {
    let dir = crate::paths::repo_target_dir_local(std::path::Path::new("/tmp"), PROJECT, REPO_PATH)
        .to_string_lossy()
        .to_string();
    let set_url =
        format!("git -C {dir} remote set-url origin https://{remote_user}@{host}/{REPO_PATH}");
    let push = rendered(&push_request(&dir, SOURCE_BRANCH, remote_user, PAT));
    Arc::new(
        ScriptedExec::new(&[])
            .with_programs(&[(set_url.as_str(), Ok("")), (push.as_str(), Ok(""))]),
    )
}

async fn publish(
    adapter: Arc<SqliteAdapter>,
    exec: Arc<ScriptedExec>,
    http: Arc<FakeHttpClient>,
    options: PublishOptions,
) {
    let publisher =
        HttpMrPublisher::with_http_override(adapter.clone(), adapter.clone(), adapter, exec, http);
    publisher
        .publish_mr_with_pat(
            PROJECT,
            &FeatureId::from(FEATURE.to_string()),
            options,
            Some(PAT),
        )
        .await
        .expect("the provider answered a created request");
}

fn options(target_branch: Option<&str>) -> PublishOptions {
    PublishOptions {
        draft: false,
        title: None,
        body: None,
        target_branch: target_branch.map(str::to_string),
    }
}

fn github_http() -> Arc<FakeHttpClient> {
    Arc::new(FakeHttpClient::new().reply(
        GITHUB_URL,
        201,
        r#"{"html_url":"https://github.com/acme/widget/pull/7","number":7,"state":"open"}"#,
    ))
}

fn gitlab_http() -> Arc<FakeHttpClient> {
    Arc::new(FakeHttpClient::new().reply(
        GITLAB_URL,
        201,
        r#"{"web_url":"https://gitlab.com/acme/widget/-/merge_requests/7","iid":7,"state":"opened"}"#,
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
async fn a_github_pr_targets_the_branch_the_caller_named() {
    let adapter = seeded("github", "github.com");
    add_feature(&adapter, FeatureOrigin::DefaultBranch);
    let http = github_http();

    publish(
        adapter,
        push_exec("x-access-token", "github.com"),
        http.clone(),
        options(Some("release/1.2")),
    )
    .await;

    assert_eq!(field(&http, GITHUB_URL, "base"), "release/1.2");
    assert_eq!(field(&http, GITHUB_URL, "head"), SOURCE_BRANCH);
}

#[tokio::test]
async fn a_gitlab_mr_targets_the_branch_the_caller_named() {
    let adapter = seeded("gitlab", "gitlab.com");
    add_feature(&adapter, FeatureOrigin::DefaultBranch);
    let http = gitlab_http();

    publish(
        adapter,
        push_exec("oauth2", "gitlab.com"),
        http.clone(),
        options(Some("release/1.2")),
    )
    .await;

    assert_eq!(field(&http, GITLAB_URL, "target_branch"), "release/1.2");
}

/// No caller names a target on the auto-publish path, so the run's own origin
/// is what stops a branch cut from `release/1.0` opening against `main`.
#[tokio::test]
async fn a_run_cut_from_a_named_base_targets_that_base() {
    let adapter = seeded("github", "github.com");
    add_feature(
        &adapter,
        FeatureOrigin::Branch {
            base: "release/1.0".to_string(),
        },
    );
    let http = github_http();

    publish(
        adapter,
        push_exec("x-access-token", "github.com"),
        http.clone(),
        options(None),
    )
    .await;

    assert_eq!(field(&http, GITHUB_URL, "base"), "release/1.0");
}
