//! The manifest flow's rules, apart from HTTP: what the manifest says, where
//! the browser is sent, and what completing it does with GitHub's answer.
//! [`start`] and [`complete`] are written against ports, so the whole flow
//! runs in a test with no Postgres, Transit or GitHub.

use super::port::GitHostStore;
use crate::adapters::github_app::{convert_manifest, ConversionError};
use crate::adapters::transit_import::rsa_key_from_pem;
use crate::auth::ceremonies::{Ceremonies, Ceremony, InsertError};
use crate::auth::RelyingParty;
use crate::ports::{KeyImporter, KeyServiceError};
use crate::store::git_hosts::{GitHostId, NewGitHost};
use crate::store::UserId;
use std::sync::Arc;

/// Where GitHub sends the browser back with `?code&state`. Outside `/v1`:
/// a top-level GET from another site, which carries no session cookie.
pub const MANIFEST_CALLBACK_PATH: &str = "/github/manifest/callback";
/// hub-web's page for git hosts, and the two outcomes it is told about.
pub const GIT_HOSTS_PAGE: &str = "/#/git-hosts";
const CONNECTED_SEGMENT: &str = "connected";
const FAILED_SEGMENT: &str = "failed";

/// GitHub refuses an App name longer than this.
const MAX_APP_NAME_CHARS: usize = 34;
/// GitHub's own limit on an account login.
const MAX_LOGIN_CHARS: usize = 39;
const TRANSIT_KEY_PREFIX: &str = "demeteo-hub-gh-";

/// A host the Hub can create an App on: github.com, and the GitHub
/// Enterprise Server its operator named. Spelled only in `Config`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectableHost {
    /// Lowercase, as a start's repository spells it.
    pub host: String,
    pub api_base: String,
    /// Where the App's pages are: `https://github.com`, or the server's root.
    pub web_base: String,
}

/// What the Hub needs to connect hosts, built once by the composition root.
pub struct GitHostsSetup {
    pub store: Arc<dyn GitHostStore>,
    pub importer: Arc<dyn KeyImporter>,
    /// For the conversion call; the same no-redirect client the minter uses.
    pub http: reqwest::Client,
    pub hosts: Vec<ConnectableHost>,
}

impl GitHostsSetup {
    pub fn host(&self, host: &str) -> Option<&ConnectableHost> {
        self.hosts
            .iter()
            .find(|h| h.host.eq_ignore_ascii_case(host))
    }
}

pub fn transit_key_name(app_id: u64) -> String {
    format!("{TRANSIT_KEY_PREFIX}{app_id}")
}

/// A GitHub login: letters, digits and single inner hyphens.
pub fn is_github_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= MAX_LOGIN_CHARS
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !login.starts_with('-')
        && !login.ends_with('-')
        && !login.contains("--")
}

/// The page GitHub creates an App from, for the user's own account or an
/// organization. `None` for an organization that is not a login, which is
/// interpolated into the path and so is never sent unchecked.
pub fn new_app_url(host: &ConnectableHost, organization: Option<&str>) -> Option<String> {
    let base = host.web_base.trim_end_matches('/');
    match organization {
        None => Some(format!("{base}/settings/apps/new")),
        Some(org) if is_github_login(org) => {
            Some(format!("{base}/organizations/{org}/settings/apps/new"))
        }
        Some(_) => None,
    }
}

/// Where the App is installed on repositories, chosen there by the user.
pub fn install_url(web_base: &str, app_slug: &str) -> String {
    format!(
        "{}/apps/{app_slug}/installations/new",
        web_base.trim_end_matches('/')
    )
}

/// The App a run needs and nothing more: private, no webhook, and exactly
/// the two repository permissions a token is minted with (`metadata: read`
/// is implied by GitHub for any App with repository permissions). `origin`
/// is the Hub's own, which GitHub sends the browser back to.
pub fn manifest(origin: &str, rp_id: &str) -> serde_json::Value {
    let name: String = format!("Demeteo {rp_id}")
        .chars()
        .take(MAX_APP_NAME_CHARS)
        .collect();
    serde_json::json!({
        "name": name,
        "url": origin,
        "description": "Pushes branches and opens pull requests for runs a Demeteo Hub starts.",
        "public": false,
        "redirect_url": format!("{origin}{MANIFEST_CALLBACK_PATH}"),
        "setup_url": format!("{origin}{GIT_HOSTS_PAGE}"),
        "setup_on_update": false,
        "request_oauth_on_install": false,
        "hook_attributes": {"url": origin, "active": false},
        "default_events": [],
        "default_permissions": {
            "contents": "write",
            "pull_requests": "write",
        },
    })
}

/// What the browser posts to GitHub: the form's `action`, its `manifest`
/// field and the `state` GitHub returns.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ManifestForm {
    pub action_url: String,
    pub manifest: String,
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StartRefusal {
    #[error("the host is not one the Hub can connect")]
    UnknownHost,
    #[error("the organization is not a GitHub login")]
    BadOrganization,
    #[error(transparent)]
    Ceremony(#[from] InsertError),
}

/// Who asked to connect a host, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartRequest<'a> {
    pub user: UserId,
    /// The passkey that signed the session in, whose budget the ceremony is
    /// held under.
    pub holder: Vec<u8>,
    /// `None` is the first connectable host, github.com.
    pub host: Option<&'a str>,
    pub organization: Option<&'a str>,
}

/// Holds a [`Ceremony::GithubManifest`] for the request's user and answers
/// the form that starts it. `rp` gives the origin GitHub returns to.
pub fn start(
    setup: &GitHostsSetup,
    ceremonies: &Ceremonies,
    rp: &RelyingParty,
    now: u64,
    request: StartRequest<'_>,
) -> Result<ManifestForm, StartRefusal> {
    let host = match request.host {
        Some(host) => setup.host(host),
        None => setup.hosts.first(),
    }
    .ok_or(StartRefusal::UnknownHost)?;
    let action_url =
        new_app_url(host, request.organization).ok_or(StartRefusal::BadOrganization)?;
    let state = ceremonies.insert(
        now,
        Ceremony::GithubManifest {
            user: request.user,
            holder: request.holder,
            host: host.host.clone(),
        },
    )?;
    Ok(ManifestForm {
        action_url,
        manifest: manifest(&rp.origin, &rp.rp_id).to_string(),
        state,
    })
}

/// Why completing failed, as the code hub-web's page is redirected with.
/// None carries GitHub's or Transit's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallbackFailure {
    /// No such `state`, or it expired or was used: start again.
    Expired,
    /// GitHub refused the code (used, expired) or could not be reached.
    Conversion,
    /// GitHub's key was not a 2048-bit RSA key.
    KeyRefused,
    /// Transit refused the import: the Hub's policy lacks the import grant.
    ImportForbidden,
    /// Transit is sealed or away.
    KeyServiceUnavailable,
    /// The key is in Transit but the row could not be written.
    StoreFailed,
}

impl CallbackFailure {
    pub fn code(self) -> &'static str {
        match self {
            CallbackFailure::Expired => "expired",
            CallbackFailure::Conversion => "conversion_failed",
            CallbackFailure::KeyRefused => "key_refused",
            CallbackFailure::ImportForbidden => "import_forbidden",
            CallbackFailure::KeyServiceUnavailable => "key_service_unavailable",
            CallbackFailure::StoreFailed => "store_failed",
        }
    }
}

/// 403 is the policy refusing the path; every other Transit failure is a
/// service to wait for.
pub fn import_failure(error: &KeyServiceError) -> CallbackFailure {
    match error {
        KeyServiceError::Upstream { status: 403, .. } => CallbackFailure::ImportForbidden,
        _ => CallbackFailure::KeyServiceUnavailable,
    }
}

pub fn connected_page(id: GitHostId) -> String {
    format!("{GIT_HOSTS_PAGE}/{CONNECTED_SEGMENT}/{}", id.0)
}

pub fn failed_page(failure: CallbackFailure) -> String {
    format!("{GIT_HOSTS_PAGE}/{FAILED_SEGMENT}/{}", failure.code())
}

/// GitHub's redirect: the `state` names the ceremony and so the tenant, the
/// `code` is exchanged for the App, the key goes to Transit and the row is
/// written. The ceremony is taken first, so a replayed redirect finds
/// nothing whatever happens after.
pub async fn complete(
    setup: &GitHostsSetup,
    ceremonies: &Ceremonies,
    now: u64,
    code: &str,
    state: &str,
) -> Result<GitHostId, CallbackFailure> {
    let Some(Ceremony::GithubManifest { user, host, .. }) = ceremonies.take(now, state) else {
        return Err(CallbackFailure::Expired);
    };
    let host = setup.host(&host).ok_or(CallbackFailure::Expired)?;
    let app = convert_manifest(&setup.http, &host.api_base, code)
        .await
        .map_err(|error| {
            if !matches!(error, ConversionError::BadCode) {
                tracing::warn!(%error, "a GitHub App manifest did not convert");
            }
            CallbackFailure::Conversion
        })?;
    let (app_id, slug, account) = (
        app.id,
        app.slug.clone(),
        app.owner.as_ref().map(|o| o.login.clone()),
    );
    let key = rsa_key_from_pem(&app.into_pem()).map_err(|refused| {
        tracing::warn!(%refused, app_id, "a converted App's key was refused");
        CallbackFailure::KeyRefused
    })?;
    let transit_key = transit_key_name(app_id);
    setup
        .importer
        .import_rsa(&transit_key, key)
        .await
        .map_err(|error| {
            tracing::warn!(%error, app_id, "a GitHub App key was not imported");
            import_failure(&error)
        })?;
    let row = NewGitHost {
        id: GitHostId::generate(),
        host: host.host.clone(),
        api_base: host.api_base.clone(),
        web_base: host.web_base.clone(),
        app_id: i64::try_from(app_id).map_err(|_| CallbackFailure::Conversion)?,
        app_slug: slug,
        transit_key,
        account,
    };
    setup.store.insert(user, &row).await.map_err(|error| {
        tracing::error!(%error, app_id, "a connected App's key is in Transit but its row is not");
        CallbackFailure::StoreFailed
    })?;
    tracing::info!(app_id, host = %row.host, "a GitHub App was connected");
    Ok(row.id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github() -> ConnectableHost {
        ConnectableHost {
            host: "github.com".to_owned(),
            api_base: "https://api.github.com".to_owned(),
            web_base: "https://github.com".to_owned(),
        }
    }

    #[test]
    fn the_new_app_page_is_the_users_own_or_an_organizations() {
        assert_eq!(
            new_app_url(&github(), None).unwrap(),
            "https://github.com/settings/apps/new"
        );
        assert_eq!(
            new_app_url(&github(), Some("octo-org")).unwrap(),
            "https://github.com/organizations/octo-org/settings/apps/new"
        );
        for org in [
            "",
            "../x",
            "a/b",
            "-a",
            "a-",
            "a--b",
            "a?b",
            &"a".repeat(40),
        ] {
            assert_eq!(new_app_url(&github(), Some(org)), None, "{org}");
        }
    }

    #[test]
    fn the_manifest_asks_for_the_two_permissions_and_no_webhook() {
        let manifest = manifest("https://hub.example", "hub.example");
        assert_eq!(manifest["public"], false);
        assert_eq!(manifest["hook_attributes"]["active"], false);
        assert_eq!(
            manifest["default_permissions"],
            serde_json::json!({"contents": "write", "pull_requests": "write"})
        );
        assert_eq!(
            manifest["redirect_url"],
            "https://hub.example/github/manifest/callback"
        );
        assert_eq!(manifest["setup_url"], "https://hub.example/#/git-hosts");
    }

    #[test]
    fn the_app_name_fits_githubs_limit() {
        let long = "a-very-long-relying-party.cluster.example.dev";
        let name = manifest("https://x", long)["name"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(name.chars().count(), MAX_APP_NAME_CHARS);
        assert!(name.starts_with("Demeteo a-very-long"));
    }

    #[test]
    fn only_a_policy_refusal_is_import_forbidden() {
        let upstream = |status| KeyServiceError::Upstream {
            status,
            detail: String::new(),
        };
        assert_eq!(
            import_failure(&upstream(403)),
            CallbackFailure::ImportForbidden
        );
        assert_eq!(
            import_failure(&upstream(400)),
            CallbackFailure::KeyServiceUnavailable
        );
        assert_eq!(
            import_failure(&KeyServiceError::Sealed),
            CallbackFailure::KeyServiceUnavailable
        );
    }

    #[test]
    fn pages_and_key_names_are_spelled_once() {
        assert_eq!(transit_key_name(5647010), "demeteo-hub-gh-5647010");
        assert!(crate::adapters::openbao::is_key_name(&transit_key_name(
            u64::MAX
        )));
        assert_eq!(
            failed_page(CallbackFailure::ImportForbidden),
            "/#/git-hosts/failed/import_forbidden"
        );
        assert_eq!(
            install_url("https://github.com/", "demeteo-x"),
            "https://github.com/apps/demeteo-x/installations/new"
        );
    }
}
