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

/// Which provider the enrichment asks, in a project holding more than one.
///
/// The listing resolves a provider per repository and the detail read did not:
/// it took `repos.first()`, so every row from the second repository onwards was
/// enriched against the first repository's host and token. Nothing surfaced it
/// — the row simply stayed undecided — so the routing is asserted here rather
/// than through a request whose failure looks identical either way.
mod routing {
    use std::sync::Arc;

    use rusqlite::Connection;

    use super::super::{FakeHttpClient, HttpMrPublisher};
    use crate::adapters::database::SqliteAdapter;
    use crate::adapters::step_executor::scripted_exec::ScriptedExec;
    use crate::domain::ids::{ProjectId, ProviderId, RepositoryId};
    use crate::domain::models::{Project, ProviderInstance, Repository};
    use crate::domain::mr_list_error::MrListError;
    use crate::ports::db::{AppSettingsRepository, ProjectRepository};

    const PROJECT: &str = "p-1";

    /// One project, and one repository per `(provider kind, host)` given — in
    /// the order given, so "the first repository" is the first entry.
    fn seeded(providers: &[(&str, &str, &str)]) -> Arc<SqliteAdapter> {
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
        for (index, (id, kind, host)) in providers.iter().enumerate() {
            adapter
                .add_provider_instance(ProviderInstance {
                    id: ProviderId::from((*id).to_string()),
                    kind: (*kind).to_string(),
                    host: (*host).to_string(),
                    username: "someone".to_string(),
                    avatar_url: String::new(),
                    created_at: 1000,
                })
                .unwrap();
            ProjectRepository::add_repository(
                adapter.as_ref(),
                Repository {
                    id: RepositoryId::from(format!("r-{index}")),
                    project_id: pid.clone(),
                    provider_id: ProviderId::from((*id).to_string()),
                    repo_path: format!("acme/widget-{index}"),
                },
            )
            .unwrap();
        }
        adapter
    }

    fn publisher(adapter: Arc<SqliteAdapter>) -> HttpMrPublisher {
        HttpMrPublisher::with_http_override(
            adapter.clone(),
            adapter.clone(),
            adapter,
            Arc::new(ScriptedExec::new(&[])),
            Arc::new(FakeHttpClient::new()),
        )
    }

    fn serving(providers: &[(&str, &str, &str)], url: &str) -> Result<String, MrListError> {
        publisher(seeded(providers))
            .provider_serving(&ProjectId::from(PROJECT.to_string()), url)
            .map(|p| p.host)
    }

    #[tokio::test]
    async fn a_row_is_enriched_against_the_host_it_came_from() {
        assert_eq!(
            serving(
                &[
                    ("prov-ghes", "github", "ghes.corp.com"),
                    ("prov-gh", "github", "github.com"),
                ],
                "https://github.com/acme/widget-1/pull/7",
            )
            .as_deref(),
            Ok("github.com"),
            "the first repository's enterprise host holds neither this request nor a token for it"
        );
    }

    #[tokio::test]
    async fn a_row_on_another_provider_kind_is_not_dispatched_at_this_one() {
        assert_eq!(
            serving(
                &[
                    ("prov-gl", "gitlab", "gitlab.com"),
                    ("prov-gh", "github", "github.com"),
                ],
                "https://github.com/acme/widget-1/pull/7",
            )
            .as_deref(),
            Ok("github.com"),
            "a GitHub URL handed to the GitLab reader is refused by its own URL parser"
        );
    }

    /// The single-provider project is every project the old resolution was
    /// right for, and a host spelled in a way `mr_route` cannot match must not
    /// cost it its enrichment.
    #[tokio::test]
    async fn one_provider_answers_for_a_url_it_does_not_look_like_it_serves() {
        assert_eq!(
            serving(
                &[("prov-gl", "gitlab", "git.internal")],
                "https://vpn.internal/acme/widget/-/merge_requests/7",
            )
            .as_deref(),
            Ok("git.internal")
        );
    }

    /// Guessing between several would send the token to a host it was not
    /// issued for, and report that host's 404 as an unreadable request.
    #[tokio::test]
    async fn several_providers_and_no_match_is_refused_rather_than_guessed() {
        let err = serving(
            &[
                ("prov-gl", "gitlab", "gitlab.com"),
                ("prov-gh", "github", "github.com"),
            ],
            "https://ghes.corp.com/acme/widget/pull/7",
        )
        .expect_err("no connected provider serves that host");

        assert!(
            matches!(&err, MrListError::Http { body, .. } if body.contains("ghes.corp.com")),
            "the refusal has to name the host nobody is connected to; got {err:?}"
        );
    }
}
