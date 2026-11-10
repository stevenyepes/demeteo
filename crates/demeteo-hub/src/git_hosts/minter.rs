//! Minting across every App that could serve a repository: the one
//! configured for the whole Hub by `DEMETEO_HUB_GITHUB_APP_ID`, and each the
//! start's tenant connected on the repository's host. Which of them mints is
//! decided by [`candidates`] and [`chosen`], both pure; [`GithubApps`] only
//! asks GitHub.

use super::port::GitHostStore;
use crate::adapters::github_app::{GithubApp, GithubAppSettings, GithubHost, Installed};
use crate::adapters::github_repos::merged;
use crate::ports::{
    GitTokenMinter, GithubRepository, InstallationToken, MintError, RepositoryLister,
    RepositoryListing, RsaSigner, UnlistedApp,
};
use crate::store::git_hosts::GitHostRow;
use crate::store::UserId;
use async_trait::async_trait;
use std::sync::Arc;

pub struct GithubApps {
    http: reqwest::Client,
    signer: Arc<dyn RsaSigner>,
    store: Arc<dyn GitHostStore>,
    configured: Option<GithubAppSettings>,
}

impl GithubApps {
    pub fn new(
        http: reqwest::Client,
        signer: Arc<dyn RsaSigner>,
        store: Arc<dyn GitHostStore>,
        configured: Option<GithubAppSettings>,
    ) -> Self {
        Self {
            http,
            signer,
            store,
            configured,
        }
    }
}

/// The Apps that could mint for a repository on `host`: the configured one
/// first when it reaches that host, then each connected row on it, an App
/// id already listed skipped. A row whose id does not fit a GitHub App id is
/// left out rather than failing every other App with it.
pub fn candidates(
    configured: Option<&GithubAppSettings>,
    rows: &[GitHostRow],
    host: &str,
) -> Vec<GithubAppSettings> {
    let mut apps: Vec<GithubAppSettings> = configured
        .filter(|app| app.hosts.iter().any(|h| h.host.eq_ignore_ascii_case(host)))
        .cloned()
        .into_iter()
        .collect();
    for row in rows
        .iter()
        .filter(|row| row.host.eq_ignore_ascii_case(host))
    {
        let Ok(app_id) = u64::try_from(row.app_id) else {
            continue;
        };
        if apps.iter().any(|app| app.app_id == app_id) {
            continue;
        }
        apps.push(GithubAppSettings {
            app_id,
            key: row.transit_key.clone(),
            hosts: vec![GithubHost {
                host: row.host.clone(),
                api_base: row.api_base.clone(),
            }],
        });
    }
    apps
}

/// One installation lookup per candidate, in order, judged together. Exactly
/// one installed mints, whatever the others answered: an App that errors
/// cannot block the one that works. Two or more installed is
/// [`MintError::Ambiguous`]. None installed reports the first failure that is
/// not a plain "not installed", since that is the one a retry may cure, and
/// otherwise [`MintError::NotInstalled`].
pub fn chosen<T>(lookups: Vec<Result<Option<T>, MintError>>) -> Result<T, MintError> {
    let mut installed = Vec::new();
    let mut failure = None;
    for lookup in lookups {
        match lookup {
            Ok(Some(found)) => installed.push(found),
            Ok(None) => {}
            Err(error) => {
                failure.get_or_insert(error);
            }
        }
    }
    match installed.len() {
        1 => Ok(installed.remove(0)),
        0 => Err(failure.unwrap_or(MintError::NotInstalled)),
        n => Err(MintError::Ambiguous(n)),
    }
}

/// Each App that could mint on each host, paired with that host: every
/// host the configured App or a connected row names, judged by
/// [`candidates`] exactly as a mint for a repository there would be.
pub fn listing_targets(
    configured: Option<&GithubAppSettings>,
    rows: &[GitHostRow],
) -> Vec<(GithubAppSettings, GithubHost)> {
    let mut hosts: Vec<String> = configured
        .into_iter()
        .flat_map(|app| app.hosts.iter().map(|h| h.host.to_ascii_lowercase()))
        .chain(rows.iter().map(|row| row.host.to_ascii_lowercase()))
        .collect();
    hosts.sort();
    hosts.dedup();
    let mut targets = Vec::new();
    for host in hosts {
        for app in candidates(configured, rows, &host) {
            let reach = app
                .hosts
                .iter()
                .find(|h| h.host.eq_ignore_ascii_case(&host))
                .cloned();
            if let Some(reach) = reach {
                targets.push((app, reach));
            }
        }
    }
    targets
}

#[async_trait]
impl RepositoryLister for GithubApps {
    async fn repositories(&self, user: UserId, now: u64) -> Result<RepositoryListing, MintError> {
        let rows = self
            .store
            .hosts(user)
            .await
            .map_err(|e| MintError::Hosts(e.to_string()))?;
        let mut listings = Vec::new();
        let mut unlisted = Vec::new();
        let mut truncated = false;
        for (settings, host) in listing_targets(self.configured.as_ref(), &rows) {
            let app_id = settings.app_id;
            let app = GithubApp::new(self.http.clone(), self.signer.clone(), settings);
            match app.repositories(&host, now).await {
                Ok(listed) => {
                    truncated |= listed.truncated;
                    listings.push(listed.repositories);
                }
                Err(error) => {
                    tracing::warn!(app_id, host = %host.host, %error, "a GitHub App's repositories could not be listed");
                    unlisted.push(UnlistedApp {
                        host: host.host.to_ascii_lowercase(),
                        app_id,
                    });
                }
            }
        }
        Ok(RepositoryListing {
            repositories: merged(listings),
            unlisted,
            truncated,
        })
    }
}

#[async_trait]
impl GitTokenMinter for GithubApps {
    async fn mint(
        &self,
        user: UserId,
        repository: &GithubRepository,
        now: u64,
    ) -> Result<InstallationToken, MintError> {
        let rows = self
            .store
            .hosts_on(user, &repository.host.to_ascii_lowercase())
            .await
            .map_err(|e| MintError::Hosts(e.to_string()))?;
        let apps = candidates(self.configured.as_ref(), &rows, &repository.host);
        if apps.is_empty() {
            return Err(MintError::UnknownHost(repository.host.clone()));
        }
        let mut lookups: Vec<Result<Option<(GithubApp, Installed)>, MintError>> = Vec::new();
        for settings in apps {
            let app = GithubApp::new(self.http.clone(), self.signer.clone(), settings);
            let found = app.installation(repository, now).await;
            lookups.push(found.map(|found| found.map(|installed| (app, installed))));
        }
        let (app, installed) = chosen(lookups)?;
        app.issue(installed, repository).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::git_hosts::GitHostId;
    use time::OffsetDateTime;

    fn configured(app_id: u64, host: &str) -> GithubAppSettings {
        GithubAppSettings {
            app_id,
            key: "demeteo-hub-github-app".to_owned(),
            hosts: vec![GithubHost {
                host: host.to_owned(),
                api_base: format!("https://api.{host}"),
            }],
        }
    }

    fn row(app_id: i64, host: &str) -> GitHostRow {
        GitHostRow {
            id: GitHostId::generate(),
            host: host.to_owned(),
            api_base: format!("https://{host}/api/v3"),
            web_base: format!("https://{host}"),
            app_id,
            app_slug: format!("app-{app_id}"),
            transit_key: format!("demeteo-hub-gh-{app_id}"),
            account: None,
            created_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    fn ids(apps: &[GithubAppSettings]) -> Vec<u64> {
        apps.iter().map(|a| a.app_id).collect()
    }

    #[test]
    fn the_configured_app_comes_first_and_a_connected_copy_of_it_is_skipped() {
        let rows = [
            row(7, "github.com"),
            row(9, "github.com"),
            row(11, "ghe.example"),
        ];
        let apps = candidates(Some(&configured(7, "github.com")), &rows, "GitHub.com");

        assert_eq!(ids(&apps), [7, 9]);
        assert_eq!(apps[0].key, "demeteo-hub-github-app");
        assert_eq!(apps[1].key, "demeteo-hub-gh-9");
        assert_eq!(apps[1].hosts[0].api_base, "https://github.com/api/v3");
    }

    #[test]
    fn every_host_is_listed_by_each_app_that_could_mint_there() {
        let rows = [
            row(7, "github.com"),
            row(9, "github.com"),
            row(11, "GHE.example"),
        ];
        let targets = listing_targets(Some(&configured(7, "github.com")), &rows);
        let shown: Vec<_> = targets
            .iter()
            .map(|(app, host)| (app.app_id, host.host.as_str()))
            .collect();

        assert_eq!(
            shown,
            [(11, "GHE.example"), (7, "github.com"), (9, "github.com")]
        );
        assert!(listing_targets(None, &[]).is_empty());
    }

    #[test]
    fn only_apps_reaching_the_host_are_candidates() {
        let rows = [
            row(9, "github.com"),
            row(11, "ghe.example"),
            row(-1, "ghe.example"),
        ];
        assert_eq!(
            ids(&candidates(
                Some(&configured(7, "github.com")),
                &rows,
                "ghe.example"
            )),
            [11]
        );
        assert!(candidates(None, &rows, "gitlab.com").is_empty());
    }

    fn not_installed() -> Result<Option<u8>, MintError> {
        Ok(None)
    }

    #[test]
    fn exactly_one_installed_app_mints_even_beside_one_that_failed() {
        let lookups = vec![
            Err(MintError::Transport("down".into())),
            Ok(Some(2u8)),
            not_installed(),
        ];
        assert_eq!(chosen(lookups).unwrap(), 2);
    }

    #[test]
    fn two_installed_apps_are_ambiguous() {
        let refused = chosen(vec![Ok(Some(1u8)), not_installed(), Ok(Some(3))]);
        assert!(
            matches!(refused, Err(MintError::Ambiguous(2))),
            "{refused:?}"
        );
    }

    #[test]
    fn none_installed_reports_the_first_real_failure_else_not_installed() {
        let failed = chosen(vec![
            not_installed(),
            Err(MintError::Refused {
                step: "x",
                status: 401,
            }),
            Err(MintError::Transport("later".into())),
        ]);
        assert!(
            matches!(failed, Err(MintError::Refused { status: 401, .. })),
            "{failed:?}"
        );
        let missing = chosen(vec![not_installed(), not_installed()]);
        assert!(
            matches!(missing, Err(MintError::NotInstalled)),
            "{missing:?}"
        );
    }
}
