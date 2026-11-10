//! The repositories a GitHub App can reach, for hub-web's project picker:
//! the App's installations listed with its JWT, then each installation's
//! repositories listed with a token that installation issues for
//! `metadata: read` alone.
//!
//! That token is a credential like the one a run gets (decision 71), so it
//! lives for the listing and is dropped: never stored, logged or returned.
//! It asks for less than a run's token, not more, so listing widens nothing
//! a connected App could already do.

use super::github_app::{answered, transport, GithubApp, GithubHost};
use crate::ports::{MintError, ReachableRepository};
use serde::Deserialize;

/// GitHub's largest page. A page shorter than this is the last one.
const PER_PAGE: usize = 100;
/// Pages read per listing before it stops: 1000 installations or 1000
/// repositories in one installation. A listing that hits it says so through
/// [`AppRepositories::truncated`] rather than reading on without bound
/// while a person waits on the page.
const MAX_PAGES: usize = 10;
const INSTALLATIONS: &str = "installation listing";
const LISTING_TOKEN: &str = "listing token request";
const REPOSITORIES: &str = "repository listing";

/// What one App answered: its repositories, and whether a cap cut them short.
#[derive(Debug, Default)]
pub struct AppRepositories {
    pub repositories: Vec<ReachableRepository>,
    pub truncated: bool,
}

pub fn installations_url(api_base: &str, page: usize) -> String {
    format!("{api_base}/app/installations?per_page={PER_PAGE}&page={page}")
}

pub fn installation_repositories_url(api_base: &str, page: usize) -> String {
    format!("{api_base}/installation/repositories?per_page={PER_PAGE}&page={page}")
}

/// No `repositories` field: the token reaches every repository the
/// installation does, which is the set being listed, and only to read
/// what GitHub calls metadata.
pub fn listing_token_body() -> serde_json::Value {
    serde_json::json!({"permissions": {"metadata": "read"}})
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
}

/// Deliberately not `Debug`: `token` is the token.
#[derive(Deserialize)]
struct ListingToken {
    token: String,
}

#[derive(Deserialize)]
struct RepositoryPage {
    repositories: Vec<WireRepository>,
}

#[derive(Deserialize)]
struct WireRepository {
    full_name: String,
    #[serde(default)]
    private: bool,
    default_branch: Option<String>,
    description: Option<String>,
}

impl GithubApp {
    /// Every repository this App's installations on `host` reach. An
    /// installation that fails fails the App: a partial list would read as
    /// "those are all of them", which is the one wrong answer a picker can
    /// give.
    pub async fn repositories(
        &self,
        host: &GithubHost,
        now: u64,
    ) -> Result<AppRepositories, MintError> {
        let jwt = self.app_jwt(now).await?;
        let mut installations = Vec::new();
        let mut truncated = false;
        for page in 1..=MAX_PAGES {
            let url = installations_url(&host.api_base, page);
            let answer = self
                .request(reqwest::Method::GET, url, &jwt)
                .send()
                .await
                .map_err(transport)?;
            let listed: Vec<Installation> = answered(answer, INSTALLATIONS).await?;
            let last = listed.len() < PER_PAGE;
            installations.extend(listed);
            if last {
                break;
            }
            truncated |= page == MAX_PAGES;
        }

        let mut repositories = Vec::new();
        for installation in installations {
            let token = self
                .listing_token(&host.api_base, installation.id, &jwt)
                .await?;
            for page in 1..=MAX_PAGES {
                let url = installation_repositories_url(&host.api_base, page);
                let answer = self
                    .request(reqwest::Method::GET, url, &token.token)
                    .send()
                    .await
                    .map_err(transport)?;
                let listed: RepositoryPage = answered(answer, REPOSITORIES).await?;
                let last = listed.repositories.len() < PER_PAGE;
                repositories.extend(listed.repositories.into_iter().map(|repo| {
                    ReachableRepository {
                        host: host.host.to_ascii_lowercase(),
                        full_name: repo.full_name,
                        private: repo.private,
                        default_branch: repo.default_branch,
                        description: repo.description,
                    }
                }));
                if last {
                    break;
                }
                truncated |= page == MAX_PAGES;
            }
        }
        Ok(AppRepositories {
            repositories,
            truncated,
        })
    }

    async fn listing_token(
        &self,
        api_base: &str,
        installation_id: u64,
        jwt: &str,
    ) -> Result<ListingToken, MintError> {
        let url = super::github_app::access_tokens_url(api_base, installation_id);
        let answer = self
            .request(reqwest::Method::POST, url, jwt)
            .json(&listing_token_body())
            .send()
            .await
            .map_err(transport)?;
        answered(answer, LISTING_TOKEN).await
    }
}

/// One list from every App's: sorted by host then name, case aside, and a
/// repository two Apps both reach listed once.
pub fn merged(listings: Vec<Vec<ReachableRepository>>) -> Vec<ReachableRepository> {
    let mut all: Vec<ReachableRepository> = listings.into_iter().flatten().collect();
    let key = |repo: &ReachableRepository| (repo.host.clone(), repo.full_name.to_ascii_lowercase());
    all.sort_by_key(key);
    all.dedup_by(|a, b| key(a) == key(b));
    all
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::github_app::tests::{answering, header};
    use crate::adapters::github_app::GithubAppSettings;
    use crate::adapters::memory_keys::MemoryKeys;
    use serde_json::{json, Value};
    use std::sync::Arc;

    const APP_KEY: &str = "github-app";
    const NOW: u64 = 1_700_000_000;

    fn repo(host: &str, full_name: &str) -> ReachableRepository {
        ReachableRepository {
            host: host.to_owned(),
            full_name: full_name.to_owned(),
            private: false,
            default_branch: None,
            description: None,
        }
    }

    #[test]
    fn a_repository_two_apps_reach_is_listed_once_in_name_order() {
        let listed = merged(vec![
            vec![
                repo("github.com", "acme/zeta"),
                repo("github.com", "acme/Widgets"),
            ],
            vec![
                repo("github.com", "acme/widgets"),
                repo("ghe.example", "acme/alpha"),
            ],
        ]);
        let names: Vec<_> = listed
            .iter()
            .map(|r| format!("{}/{}", r.host, r.full_name))
            .collect();
        assert_eq!(
            names,
            [
                "ghe.example/acme/alpha",
                "github.com/acme/Widgets",
                "github.com/acme/zeta"
            ]
        );
    }

    #[test]
    fn the_listing_token_asks_for_metadata_and_nothing_else() {
        assert_eq!(
            listing_token_body(),
            json!({"permissions": {"metadata": "read"}})
        );
        assert_eq!(
            installations_url("https://api.github.com", 2),
            "https://api.github.com/app/installations?per_page=100&page=2"
        );
        assert_eq!(
            installation_repositories_url("https://api.github.com", 1),
            "https://api.github.com/installation/repositories?per_page=100&page=1"
        );
    }

    fn app_on(base: &str) -> (GithubApp, GithubHost) {
        let host = GithubHost {
            host: "GitHub.com".to_owned(),
            api_base: base.to_owned(),
        };
        let keys = Arc::new(MemoryKeys::with_keys(["other"]).with_rsa_key(APP_KEY));
        let app = GithubApp::new(
            reqwest::Client::new(),
            keys,
            GithubAppSettings {
                app_id: 424_242,
                key: APP_KEY.to_owned(),
                hosts: vec![host.clone()],
            },
        );
        (app, host)
    }

    fn page(repositories: Value) -> Value {
        json!({"total_count": 1, "repositories": repositories})
    }

    fn minted(id: u64) -> (u16, Value) {
        (
            201,
            json!({"token": format!("ghs_{id}"), "expires_at": "2030-01-01T00:00:00Z"}),
        )
    }

    #[tokio::test]
    async fn each_installation_lists_its_repositories_with_its_own_metadata_token() {
        let (base, served) = answering(vec![
            (200, json!([{"id": 7}, {"id": 8}])),
            minted(7),
            (
                200,
                page(json!([{"full_name": "acme/widgets", "private": true, "default_branch": "main", "description": "Widgets"}])),
            ),
            minted(8),
            (
                200,
                page(json!([{"full_name": "octo/site", "private": false, "default_branch": null, "description": null}])),
            ),
        ])
        .await;
        let (app, host) = app_on(&base);

        let listed = app.repositories(&host, NOW).await.unwrap();

        assert!(!listed.truncated);
        assert_eq!(
            listed.repositories,
            vec![
                ReachableRepository {
                    host: "github.com".to_owned(),
                    full_name: "acme/widgets".to_owned(),
                    private: true,
                    default_branch: Some("main".to_owned()),
                    description: Some("Widgets".to_owned()),
                },
                repo("github.com", "octo/site"),
            ]
        );
        let requests = served.await.unwrap();
        let lines: Vec<&str> = requests
            .iter()
            .map(|r| r.lines().next().unwrap_or_default())
            .collect();
        assert_eq!(
            lines,
            [
                "GET /app/installations?per_page=100&page=1 HTTP/1.1",
                "POST /app/installations/7/access_tokens HTTP/1.1",
                "GET /installation/repositories?per_page=100&page=1 HTTP/1.1",
                "POST /app/installations/8/access_tokens HTTP/1.1",
                "GET /installation/repositories?per_page=100&page=1 HTTP/1.1",
            ]
        );
        assert_eq!(header(&requests[2], "authorization"), Some("Bearer ghs_7"));
        assert_eq!(header(&requests[4], "authorization"), Some("Bearer ghs_8"));
        for token_request in [&requests[1], &requests[3]] {
            let (_, body) = token_request.split_once("\r\n\r\n").unwrap();
            assert_eq!(
                serde_json::from_str::<Value>(body).unwrap(),
                listing_token_body()
            );
        }
    }

    #[tokio::test]
    async fn one_installation_refusing_a_token_fails_the_app_without_its_body() {
        let (base, _served) = answering(vec![
            (200, json!([{"id": 7}, {"id": 8}])),
            minted(7),
            (200, page(json!([]))),
            (403, json!({"message": "ghs_nope"})),
        ])
        .await;
        let (app, host) = app_on(&base);

        let refused = app.repositories(&host, NOW).await.unwrap_err();

        match &refused {
            MintError::Refused { step, status: 403 } => assert_eq!(*step, LISTING_TOKEN),
            other => panic!("{other:?}"),
        }
        assert!(!refused.to_string().contains("ghs_nope"), "{refused}");
    }
}
