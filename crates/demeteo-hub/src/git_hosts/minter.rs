//! Minting across every App that could serve a repository: the one
//! configured for the whole Hub by `DEMETEO_HUB_GITHUB_APP_ID`, and each the
//! start's tenant connected on the repository's host. Which of them mints is
//! decided by [`candidates`] and [`chosen`], both pure; [`GithubApps`] only
//! asks GitHub.

use super::port::GitHostStore;
use crate::adapters::github_app::{GithubApp, GithubAppSettings, GithubHost, Installed};
use crate::ports::{GitTokenMinter, GithubRepository, InstallationToken, MintError, RsaSigner};
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
