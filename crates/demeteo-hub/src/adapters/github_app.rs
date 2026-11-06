//! Minting a GitHub App installation token for one repository: an App JWT
//! signed through Transit, the repository's installation looked up with it,
//! and a token asked of that installation for the one repository with
//! `contents: write` and `pull_requests: write` (decision 71).
//!
//! The App's private key never enters this process: [`RsaSigner`] is handed
//! the JWT's signing input and answers the signature. What is built here
//! from bytes, the JWT and the URLs, is built by free functions, so the
//! exact bytes are reachable from a test with no server.

use crate::ports::{GithubRepository, InstallationToken, MintError, RsaSigner};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use time::{Date, Month, PrimitiveDateTime, Time};

const JWT_HEADER: &str = r#"{"alg":"RS256","typ":"JWT"}"#;
/// GitHub refuses an App JWT that lives longer than ten minutes, and one
/// issued by a clock ahead of its own. Backdating `iat` a minute absorbs
/// that skew; `exp` stays inside the ten minutes counted from the backdated
/// `iat`.
const JWT_BACKDATE_SECS: u64 = 60;
const JWT_LIFETIME_SECS: u64 = 540;
const MEDIA_TYPE: &str = "application/vnd.github+json";
const API_VERSION_HEADER: &str = "X-GitHub-Api-Version";
const API_VERSION: &str = "2022-11-28";
const AGENT: &str = concat!("demeteo-hub/", env!("CARGO_PKG_VERSION"));
const LOOKUP: &str = "installation lookup";
const TOKEN_REQUEST: &str = "token request";

/// A host repositories are cloned from, and the REST API that serves it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubHost {
    /// Lowercase, with a port when the clone URL names one: the spelling
    /// [`RunnerRepository::host`](demeteo_hub_protocol::RunnerRepository)
    /// takes, compared without regard to ASCII case.
    pub host: String,
    /// No trailing `/`.
    pub api_base: String,
}

/// The App the Hub mints with. Read from `Config`, which is the only place a
/// host may be spelled; `hosts` always holds github.com.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubAppSettings {
    pub app_id: u64,
    /// The Transit name of the App's imported RSA key.
    pub key: String,
    pub hosts: Vec<GithubHost>,
}

/// The API serving `host`, or `None` for a host no setting names.
pub fn api_base_for<'a>(hosts: &'a [GithubHost], host: &str) -> Option<&'a str> {
    hosts
        .iter()
        .find(|known| known.host.eq_ignore_ascii_case(host))
        .map(|known| known.api_base.as_str())
}

/// The JWT's first two segments, which are what is signed.
pub fn jwt_signing_input(app_id: u64, now: u64) -> String {
    let iat = now.saturating_sub(JWT_BACKDATE_SECS);
    let exp = now.saturating_add(JWT_LIFETIME_SECS);
    let claims = format!(r#"{{"iat":{iat},"exp":{exp},"iss":{app_id}}}"#);
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(JWT_HEADER),
        URL_SAFE_NO_PAD.encode(claims)
    )
}

pub fn jwt(signing_input: &str, signature: &[u8]) -> String {
    format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature))
}

/// `YYYY-MM-DDTHH:MM:SSZ`, the one shape GitHub answers `expires_at` in, as
/// Unix seconds. Anything else is `None` rather than a guess at an expiry: a
/// runner handed a wrong one would hold a token past its life or refresh one
/// that was fine.
pub fn unix_of_iso8601(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    let shaped = bytes.len() == 20
        && bytes.iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            10 => *b == b'T',
            13 | 16 => *b == b':',
            19 => *b == b'Z',
            _ => b.is_ascii_digit(),
        });
    if !shaped {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<u16>().ok();
    let year = i32::from(number(0..4)?);
    let month = Month::try_from(u8::try_from(number(5..7)?).ok()?).ok()?;
    let day = u8::try_from(number(8..10)?).ok()?;
    let hour = u8::try_from(number(11..13)?).ok()?;
    let minute = u8::try_from(number(14..16)?).ok()?;
    let second = u8::try_from(number(17..19)?).ok()?;
    let at = PrimitiveDateTime::new(
        Date::from_calendar_date(year, month, day).ok()?,
        Time::from_hms(hour, minute, second).ok()?,
    );
    u64::try_from(at.assume_utc().unix_timestamp()).ok()
}

/// `owner` and `name` are path segments: the git-token verdict holds them to
/// GitHub's own name alphabet before a minter is asked, so neither can
/// carry a `/`, `?` or `#` into another route.
pub fn installation_url(api_base: &str, repository: &GithubRepository) -> String {
    format!(
        "{api_base}/repos/{}/{}/installation",
        repository.owner, repository.name
    )
}

pub fn access_tokens_url(api_base: &str, installation_id: u64) -> String {
    format!("{api_base}/app/installations/{installation_id}/access_tokens")
}

#[derive(Serialize)]
struct TokenRequest<'a> {
    repositories: [&'a str; 1],
    permissions: Permissions,
}

#[derive(Serialize)]
struct Permissions {
    contents: &'static str,
    pull_requests: &'static str,
}

/// The body of the token request: the one repository, and no permission
/// beyond the two a run pushes and opens its pull request with.
pub fn token_request_body(repository: &GithubRepository) -> serde_json::Value {
    let body = TokenRequest {
        repositories: [&repository.name],
        permissions: Permissions {
            contents: "write",
            pull_requests: "write",
        },
    };
    serde_json::json!(body)
}

#[derive(Deserialize)]
struct Installation {
    id: u64,
}

/// Deliberately not `Debug`: `token` is the token.
#[derive(Deserialize)]
struct AccessToken {
    token: String,
    expires_at: String,
}

/// The client a [`GithubApp`] should be built over. Redirects are not
/// followed: an API that moves is a misconfigured base, and a JWT must not
/// travel to wherever a `Location` points.
pub fn http_client(connect: Duration, total: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(connect)
        .timeout(total)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

pub struct GithubApp {
    http: reqwest::Client,
    signer: Arc<dyn RsaSigner>,
    settings: GithubAppSettings,
}

impl GithubApp {
    pub fn new(
        http: reqwest::Client,
        signer: Arc<dyn RsaSigner>,
        settings: GithubAppSettings,
    ) -> Self {
        Self {
            http,
            signer,
            settings,
        }
    }

    async fn app_jwt(&self, now: u64) -> Result<String, MintError> {
        let input = jwt_signing_input(self.settings.app_id, now);
        let signature = self
            .signer
            .sign_rs256(&self.settings.key, input.as_bytes())
            .await
            .map_err(MintError::Signing)?;
        Ok(jwt(&input, &signature))
    }

    fn request(&self, method: reqwest::Method, url: String, jwt: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, url)
            .header(AUTHORIZATION, format!("Bearer {jwt}"))
            .header(ACCEPT, MEDIA_TYPE)
            .header(API_VERSION_HEADER, API_VERSION)
            .header(USER_AGENT, AGENT)
    }
}

/// This App's installation on one repository, and the App JWT that found
/// it, which the token request reuses. `Debug` omits the JWT: it is a
/// credential for its ten minutes.
pub struct Installed {
    id: u64,
    jwt: String,
    api_base: String,
}

impl std::fmt::Debug for Installed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Installed")
            .field("id", &self.id)
            .field("api_base", &self.api_base)
            .finish_non_exhaustive()
    }
}

impl GithubApp {
    pub fn settings(&self) -> &GithubAppSettings {
        &self.settings
    }

    /// `None` when the App is not installed on the repository: GitHub's
    /// lookup answers 404, which also covers an App id naming another App.
    pub async fn installation(
        &self,
        repository: &GithubRepository,
        now: u64,
    ) -> Result<Option<Installed>, MintError> {
        let api_base = api_base_for(&self.settings.hosts, &repository.host)
            .ok_or_else(|| MintError::UnknownHost(repository.host.clone()))?;
        let jwt = self.app_jwt(now).await?;

        let lookup = self
            .request(
                reqwest::Method::GET,
                installation_url(api_base, repository),
                &jwt,
            )
            .send()
            .await
            .map_err(transport)?;
        if lookup.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let installation: Installation = answered(lookup, LOOKUP).await?;
        Ok(Some(Installed {
            id: installation.id,
            jwt,
            api_base: api_base.to_owned(),
        }))
    }

    /// A one-App mint: the installation, then its token.
    pub async fn mint(
        &self,
        repository: &GithubRepository,
        now: u64,
    ) -> Result<InstallationToken, MintError> {
        let installed = self
            .installation(repository, now)
            .await?
            .ok_or(MintError::NotInstalled)?;
        self.issue(installed, repository).await
    }

    pub async fn issue(
        &self,
        installed: Installed,
        repository: &GithubRepository,
    ) -> Result<InstallationToken, MintError> {
        let issued = self
            .request(
                reqwest::Method::POST,
                access_tokens_url(&installed.api_base, installed.id),
                &installed.jwt,
            )
            .json(&token_request_body(repository))
            .send()
            .await
            .map_err(transport)?;
        let token: AccessToken = answered(issued, TOKEN_REQUEST).await?;
        let expires_at =
            unix_of_iso8601(&token.expires_at).ok_or(MintError::Malformed(TOKEN_REQUEST))?;
        Ok(InstallationToken::new(token.token, expires_at))
    }
}

/// What GitHub hands back once, from the manifest conversion: the new App's
/// id and slug, the account that owns it, and its private key. The client
/// and webhook secrets in the same answer are never deserialised, so they
/// cannot be kept by accident. `Debug` omits the key.
#[derive(Deserialize)]
pub struct ConvertedApp {
    pub id: u64,
    pub slug: String,
    pub owner: Option<AppOwner>,
    pem: String,
}

#[derive(Debug, Deserialize)]
pub struct AppOwner {
    pub login: String,
}

impl ConvertedApp {
    /// The private key, consumed: the caller imports it and keeps nothing.
    pub fn into_pem(self) -> String {
        self.pem
    }
}

impl std::fmt::Debug for ConvertedApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConvertedApp")
            .field("id", &self.id)
            .field("slug", &self.slug)
            .field("owner", &self.owner)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConversionError {
    /// The `code` GitHub redirected with is not one it could have issued, so
    /// it is not sent: it is interpolated into the URL path.
    #[error("the manifest code is not a code")]
    BadCode,
    #[error("GitHub could not be reached: {0}")]
    Transport(String),
    /// 404 is a code already used or older than its hour.
    #[error("GitHub answered HTTP {0} to the manifest conversion")]
    Refused(u16),
    #[error("GitHub answered the manifest conversion with a body this build cannot read")]
    Malformed,
}

const MAX_CODE_LEN: usize = 128;

pub fn conversion_url(api_base: &str, code: &str) -> Option<String> {
    let a_code = !code.is_empty()
        && code.len() <= MAX_CODE_LEN
        && code.bytes().all(|b| b.is_ascii_alphanumeric());
    a_code.then(|| format!("{api_base}/app-manifests/{code}/conversions"))
}

/// Exchanges the one-hour code GitHub redirected back with for the App it
/// created. Unauthenticated by GitHub's design: the code is the credential,
/// and it works once.
pub async fn convert_manifest(
    http: &reqwest::Client,
    api_base: &str,
    code: &str,
) -> Result<ConvertedApp, ConversionError> {
    let url = conversion_url(api_base, code).ok_or(ConversionError::BadCode)?;
    let response = http
        .post(url)
        .header(ACCEPT, MEDIA_TYPE)
        .header(API_VERSION_HEADER, API_VERSION)
        .header(USER_AGENT, AGENT)
        .send()
        .await
        .map_err(|e| ConversionError::Transport(e.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(ConversionError::Refused(status.as_u16()));
    }
    let body = response
        .bytes()
        .await
        .map_err(|e| ConversionError::Transport(e.without_url().to_string()))?;
    serde_json::from_slice(&body).map_err(|_| ConversionError::Malformed)
}

/// A 2xx body read as `T`. Any other status is reported by its number
/// alone, and its body is never read.
async fn answered<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    step: &'static str,
) -> Result<T, MintError> {
    let status = response.status();
    if !status.is_success() {
        return Err(MintError::Refused {
            step,
            status: status.as_u16(),
        });
    }
    let body = response.bytes().await.map_err(transport)?;
    serde_json::from_slice(&body).map_err(|_| MintError::Malformed(step))
}

fn transport(error: reqwest::Error) -> MintError {
    MintError::Transport(error.without_url().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::memory_keys::MemoryKeys;
    use aws_lc_rs::signature::{UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};
    use serde_json::{json, Value};

    const APP_KEY: &str = "github-app";
    const NOW: u64 = 1_700_000_000;

    fn repository(host: &str) -> GithubRepository {
        GithubRepository {
            host: host.to_owned(),
            owner: "acme".to_owned(),
            name: "widgets".to_owned(),
        }
    }

    fn segment(text: &str) -> Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(text).unwrap()).unwrap()
    }

    #[test]
    fn the_jwt_is_rs256_issued_a_minute_back_for_nine_minutes_by_the_app() {
        let input = jwt_signing_input(424_242, NOW);
        let (header, claims) = input.split_once('.').unwrap();

        assert_eq!(
            URL_SAFE_NO_PAD.decode(header).unwrap(),
            br#"{"alg":"RS256","typ":"JWT"}"#
        );
        assert_eq!(
            URL_SAFE_NO_PAD.decode(claims).unwrap(),
            br#"{"iat":1699999940,"exp":1700000540,"iss":424242}"#
        );
        assert!(!input.contains('='), "{input}");
        assert_eq!(
            jwt(&input, &[0xfb, 0xff]),
            format!("{input}.-_8"),
            "the signature is base64url, unpadded"
        );
    }

    #[test]
    fn an_iso8601_utc_second_is_its_unix_second() {
        assert_eq!(unix_of_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(unix_of_iso8601("2016-07-11T22:14:10Z"), Some(1_468_275_250));
        assert_eq!(unix_of_iso8601("2024-02-29T23:59:59Z"), Some(1_709_251_199));
    }

    #[test]
    fn an_expiry_in_any_other_shape_is_none() {
        for text in [
            "",
            "2016-07-11T22:14:10",
            "2016-07-11T22:14:10z",
            "2016-07-11T22:14:100",
            "2016-07-11T22:14:10+00:00",
            "2016-07-11T22:14:10.123Z",
            "2016-07-11 22:14:10Z",
            "2016-13-11T22:14:10Z",
            "2023-02-29T22:14:10Z",
            "2016-07-11T24:14:10Z",
            "+016-07-11T22:14:10Z",
            "1969-12-31T23:59:59Z",
            "2016-07-11T22:14:1٠Z",
        ] {
            assert_eq!(unix_of_iso8601(text), None, "{text}");
        }
    }

    #[test]
    fn a_host_is_found_without_regard_to_case_and_only_if_named() {
        let hosts = vec![
            GithubHost {
                host: "github.com".to_owned(),
                api_base: "https://api.github.com".to_owned(),
            },
            GithubHost {
                host: "ghe.example:8443".to_owned(),
                api_base: "https://ghe.example:8443/api/v3".to_owned(),
            },
        ];

        assert_eq!(
            api_base_for(&hosts, "GitHub.com"),
            Some("https://api.github.com")
        );
        assert_eq!(
            api_base_for(&hosts, "ghe.example:8443"),
            Some("https://ghe.example:8443/api/v3")
        );
        assert_eq!(api_base_for(&hosts, "ghe.example"), None);
        assert_eq!(api_base_for(&hosts, "gitlab.com"), None);
    }

    #[test]
    fn the_token_asked_for_reaches_one_repository_with_two_permissions() {
        assert_eq!(
            token_request_body(&repository("github.com")),
            json!({
                "repositories": ["widgets"],
                "permissions": {"contents": "write", "pull_requests": "write"},
            })
        );
        assert_eq!(
            installation_url("https://api.github.com", &repository("github.com")),
            "https://api.github.com/repos/acme/widgets/installation"
        );
        assert_eq!(
            access_tokens_url("https://api.github.com", 77),
            "https://api.github.com/app/installations/77/access_tokens"
        );
    }

    #[test]
    fn debug_never_prints_a_minted_token() {
        let token = InstallationToken::new("ghs_sekret".to_owned(), 9);
        let shown = format!("{token:?}");
        assert!(!shown.contains("sekret"), "{shown}");
        assert_eq!(token.secret(), "ghs_sekret");
    }

    /// Serves `answers` to as many connections, one each, in order, and
    /// returns every request it read.
    async fn answering(
        answers: Vec<(u16, Value)>,
    ) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let served = tokio::spawn(async move {
            let mut requests = Vec::new();
            for (status, body) in answers {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut seen = Vec::new();
                let mut chunk = [0u8; 4096];
                let request = loop {
                    let n = socket.read(&mut chunk).await.unwrap();
                    seen.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&seen).into_owned();
                    let complete = text.split_once("\r\n\r\n").is_some_and(|(head, sent)| {
                        let declared = head
                            .lines()
                            .filter_map(|line| line.split_once(':'))
                            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        sent.len() >= declared
                    });
                    if complete || n == 0 {
                        break text;
                    }
                };
                let body = serde_json::to_vec(&body).unwrap();
                let head = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(head.as_bytes()).await.unwrap();
                socket.write_all(&body).await.unwrap();
                requests.push(request);
            }
            requests
        });
        (format!("http://{addr}"), served)
    }

    fn app(api_base: &str, keys: Arc<MemoryKeys>) -> GithubApp {
        GithubApp::new(
            reqwest::Client::new(),
            keys,
            GithubAppSettings {
                app_id: 424_242,
                key: APP_KEY.to_owned(),
                hosts: vec![GithubHost {
                    host: "github.com".to_owned(),
                    api_base: api_base.to_owned(),
                }],
            },
        )
    }

    fn header<'a>(request: &'a str, name: &str) -> Option<&'a str> {
        request
            .split_once("\r\n\r\n")?
            .0
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.trim())
    }

    #[tokio::test]
    async fn a_token_is_minted_with_a_jwt_the_app_key_signed() {
        let (base, served) = answering(vec![
            (200, json!({"id": 77, "account": {"login": "acme"}})),
            (
                201,
                json!({"token": "ghs_minted", "expires_at": "2016-07-11T22:14:10Z"}),
            ),
        ])
        .await;
        let keys = Arc::new(MemoryKeys::with_keys(["other"]).with_rsa_key(APP_KEY));

        let token = app(&base, Arc::clone(&keys))
            .mint(&repository("GITHUB.com"), NOW)
            .await
            .unwrap();

        assert_eq!(token.secret(), "ghs_minted");
        assert_eq!(token.expires_at, 1_468_275_250);
        let requests = served.await.unwrap();
        assert!(
            requests[0].starts_with("GET /repos/acme/widgets/installation HTTP/1.1\r\n"),
            "{}",
            requests[0]
        );
        assert!(
            requests[1].starts_with("POST /app/installations/77/access_tokens HTTP/1.1\r\n"),
            "{}",
            requests[1]
        );
        let (_, body) = requests[1].split_once("\r\n\r\n").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap(),
            token_request_body(&repository("github.com"))
        );
        for request in &requests {
            assert_eq!(header(request, "accept"), Some(MEDIA_TYPE));
            assert_eq!(header(request, "x-github-api-version"), Some(API_VERSION));
            assert!(header(request, "user-agent").is_some_and(|ua| ua.starts_with("demeteo-hub/")));
            let jwt = header(request, "authorization")
                .and_then(|value| value.strip_prefix("Bearer "))
                .unwrap();
            let (input, signature) = jwt.rsplit_once('.').unwrap();
            assert_eq!(input, jwt_signing_input(424_242, NOW));
            assert_eq!(segment(input.split_once('.').unwrap().1)["iss"], 424_242);
            UnparsedPublicKey::new(
                &RSA_PKCS1_2048_8192_SHA256,
                keys.rsa_public_key(APP_KEY).unwrap(),
            )
            .verify(
                input.as_bytes(),
                &URL_SAFE_NO_PAD.decode(signature).unwrap(),
            )
            .unwrap();
        }
    }

    #[tokio::test]
    async fn an_app_not_installed_on_the_repository_asks_for_no_token() {
        let (base, served) = answering(vec![(404, json!({"message": "Not Found"}))]).await;
        let keys = Arc::new(MemoryKeys::with_keys(["other"]).with_rsa_key(APP_KEY));

        let refused = app(&base, keys).mint(&repository("github.com"), NOW).await;

        assert!(
            matches!(refused, Err(MintError::NotInstalled)),
            "{refused:?}"
        );
        assert_eq!(served.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_refused_token_request_is_its_status_and_not_its_body() {
        let (base, _served) = answering(vec![
            (200, json!({"id": 77})),
            (422, json!({"message": "ghs_leaked_in_an_error"})),
        ])
        .await;
        let keys = Arc::new(MemoryKeys::with_keys(["other"]).with_rsa_key(APP_KEY));

        let refused = app(&base, keys).mint(&repository("github.com"), NOW).await;

        match refused {
            Err(error @ MintError::Refused { step, status: 422 }) => {
                assert_eq!(step, TOKEN_REQUEST);
                assert!(!error.to_string().contains("leaked"), "{error}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreadable_expiry_is_malformed_and_hands_no_token_over() {
        let (base, _served) = answering(vec![
            (200, json!({"id": 77})),
            (
                201,
                json!({"token": "ghs_minted", "expires_at": "in an hour"}),
            ),
        ])
        .await;
        let keys = Arc::new(MemoryKeys::with_keys(["other"]).with_rsa_key(APP_KEY));

        let refused = app(&base, keys).mint(&repository("github.com"), NOW).await;

        assert!(
            matches!(refused, Err(MintError::Malformed(TOKEN_REQUEST))),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn neither_an_unknown_host_nor_a_sealed_signer_reaches_github() {
        let unreachable = "http://github.invalid";
        let keys = Arc::new(MemoryKeys::with_keys(["other"]).with_rsa_key(APP_KEY));

        let unknown = app(unreachable, keys)
            .mint(&repository("gitlab.com"), NOW)
            .await;
        let sealed = app(unreachable, Arc::new(MemoryKeys::sealed()))
            .mint(&repository("github.com"), NOW)
            .await;

        assert!(
            matches!(&unknown, Err(MintError::UnknownHost(host)) if host == "gitlab.com"),
            "{unknown:?}"
        );
        assert!(
            matches!(
                sealed,
                Err(MintError::Signing(crate::ports::KeyServiceError::Sealed))
            ),
            "{sealed:?}"
        );
    }

    #[tokio::test]
    async fn a_manifest_code_converts_to_the_app_without_its_other_secrets() {
        let (base, served) = answering(vec![(
            201,
            serde_json::json!({
                "id": 5647010,
                "slug": "demeteo-hub-example",
                "owner": {"login": "octo-org"},
                "client_id": "Iv23li",
                "client_secret": "client-secret",
                "webhook_secret": "webhook-secret",
                "pem": "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n",
            }),
        )])
        .await;

        let app = convert_manifest(&reqwest::Client::new(), &base, "a1b2c3")
            .await
            .unwrap();
        let requests = served.await.unwrap();

        assert!(requests[0].starts_with("POST /app-manifests/a1b2c3/conversions HTTP/1.1"));
        assert!(!requests[0].to_ascii_lowercase().contains("authorization:"));
        assert_eq!(
            (app.id, app.slug.as_str()),
            (5647010, "demeteo-hub-example")
        );
        assert_eq!(
            app.owner.as_ref().map(|o| o.login.as_str()),
            Some("octo-org")
        );
        let debug = format!("{app:?}");
        assert!(
            !debug.contains("PRIVATE KEY") && !debug.contains("secret"),
            "{debug}"
        );
        assert!(app
            .into_pem()
            .starts_with("-----BEGIN RSA PRIVATE KEY-----"));
    }

    #[tokio::test]
    async fn a_used_code_is_refused_and_a_code_that_is_no_code_is_never_sent() {
        let (base, _served) = answering(vec![(404, serde_json::json!({}))]).await;
        let http = reqwest::Client::new();

        assert_eq!(
            convert_manifest(&http, &base, "used").await.unwrap_err(),
            ConversionError::Refused(404)
        );
        for code in ["", "../app", "a?b", &"a".repeat(MAX_CODE_LEN + 1)] {
            assert_eq!(
                convert_manifest(&http, "http://github.invalid", code)
                    .await
                    .unwrap_err(),
                ConversionError::BadCode,
                "{code}"
            );
        }
    }
}
