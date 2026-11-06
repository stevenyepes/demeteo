//! How the Hub proves itself to OpenBao: a token an operator minted and
//! renews, or a login with the Hub's Kubernetes service account.
//!
//! The Kubernetes login exists so that no OpenBao token has to be minted,
//! stored in a Secret, or renewed by a job: the Hub holds one only in memory,
//! for the role's TTL, and logs in again before it runs out. The service
//! account token is read from its file at every login, because a projected
//! token is rotated by the kubelet while the pod runs.

use crate::ports::KeyServiceError;
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;

const STATUS_SEALED: u16 = 503;

#[derive(Clone, PartialEq, Eq)]
pub enum OpenBaoAuth {
    Token(String),
    Kubernetes(KubernetesLogin),
}

impl fmt::Debug for OpenBaoAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Token(_) => f.write_str("Token(<redacted>)"),
            Self::Kubernetes(login) => f.debug_tuple("Kubernetes").field(login).finish(),
        }
    }
}

/// None of this is a secret: the role and mount are names, and the file is
/// where the kubelet puts the token, not the token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KubernetesLogin {
    pub role: String,
    pub mount: String,
    pub jwt_file: PathBuf,
}

/// When a token whose lease is `lease_secs` long, issued at `now`, is
/// replaced: at two thirds of its life, so a slow login still lands before
/// the old token expires. A lease of `0` is a token that does not expire.
pub fn renew_at(now: u64, lease_secs: u64) -> u64 {
    if lease_secs == 0 {
        u64::MAX
    } else {
        now.saturating_add(lease_secs.saturating_mul(2) / 3)
    }
}

pub fn needs_login(now: u64, held_until: Option<u64>) -> bool {
    held_until.is_none_or(|renew_at| now >= renew_at)
}

pub(crate) enum Credential {
    Static(HeaderValue),
    Login(Box<LoginState>),
}

pub(crate) struct LoginState {
    http: reqwest::Client,
    url: String,
    role: String,
    jwt_file: PathBuf,
    /// Held across the login itself, so concurrent requests that find the
    /// token stale wait for one login instead of each starting their own.
    held: Mutex<Option<Held>>,
}

struct Held {
    header: HeaderValue,
    renew_at: u64,
}

#[derive(Serialize)]
struct LoginBody<'a> {
    role: &'a str,
    jwt: &'a str,
}

#[derive(Deserialize)]
struct LoginAnswer {
    auth: LoginAuth,
}

#[derive(Deserialize)]
struct LoginAuth {
    client_token: String,
    lease_duration: u64,
}

#[derive(Deserialize)]
struct LoginErrors {
    errors: Vec<String>,
}

impl Credential {
    /// `mount` must already be a checked path, as the Transit mount is.
    pub(crate) fn new(
        http: &reqwest::Client,
        addr: &str,
        mount: &str,
        auth: &OpenBaoAuth,
    ) -> Result<Self, KeyServiceError> {
        match auth {
            OpenBaoAuth::Token(token) => Ok(Self::Static(token_header(token)?)),
            OpenBaoAuth::Kubernetes(login) => Ok(Self::Login(Box::new(LoginState {
                http: http.clone(),
                url: login_url(addr, mount),
                role: login.role.clone(),
                jwt_file: login.jwt_file.clone(),
                held: Mutex::new(None),
            }))),
        }
    }

    pub(crate) async fn header(&self) -> Result<HeaderValue, KeyServiceError> {
        match self {
            Self::Static(header) => Ok(header.clone()),
            Self::Login(state) => state.header(unix_now()).await,
        }
    }

    /// Drops `used` after OpenBao refused it, and says whether a retry could
    /// fare better. A static token cannot: the same token would be sent.
    pub(crate) async fn forget(&self, used: &HeaderValue) -> bool {
        match self {
            Self::Static(_) => false,
            Self::Login(state) => {
                let mut held = state.held.lock().await;
                if held.as_ref().is_some_and(|h| h.header == used) {
                    *held = None;
                }
                true
            }
        }
    }
}

impl LoginState {
    async fn header(&self, now: u64) -> Result<HeaderValue, KeyServiceError> {
        let mut held = self.held.lock().await;
        if let Some(current) = held
            .as_ref()
            .filter(|h| !needs_login(now, Some(h.renew_at)))
        {
            return Ok(current.header.clone());
        }
        let fresh = self.login(now).await?;
        let header = fresh.header.clone();
        *held = Some(fresh);
        Ok(header)
    }

    async fn login(&self, now: u64) -> Result<Held, KeyServiceError> {
        let jwt = tokio::fs::read_to_string(&self.jwt_file)
            .await
            .map_err(|e| {
                KeyServiceError::Transport(format!("cannot read the service account token: {e}"))
            })?;
        let body = LoginBody {
            role: &self.role,
            jwt: jwt.trim(),
        };
        let response = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| KeyServiceError::Transport(e.without_url().to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .bytes()
            .await
            .map_err(|e| KeyServiceError::Transport(e.without_url().to_string()))?;
        if !(200..300).contains(&status) {
            return Err(login_failure(status, &bytes));
        }
        let answer = serde_json::from_slice::<LoginAnswer>(&bytes)
            .map_err(|e| KeyServiceError::Malformed(format!("login answer: {e}")))?;
        Ok(Held {
            header: token_header(&answer.auth.client_token)?,
            renew_at: renew_at(now, answer.auth.lease_duration),
        })
    }
}

pub(crate) fn token_header(token: &str) -> Result<HeaderValue, KeyServiceError> {
    let mut header = HeaderValue::from_str(token)
        .map_err(|_| KeyServiceError::Transport("token is not a valid header value".into()))?;
    header.set_sensitive(true);
    Ok(header)
}

fn login_url(addr: &str, mount: &str) -> String {
    format!(
        "{}/v1/auth/{}/login",
        addr.trim_end_matches('/'),
        mount.trim_matches('/')
    )
}

/// A refused login is `Upstream` with OpenBao's own words, which name the
/// role or the audience that did not match; never the JWT, which is only in
/// the request.
fn login_failure(status: u16, body: &[u8]) -> KeyServiceError {
    if status == STATUS_SEALED {
        return KeyServiceError::Sealed;
    }
    let detail = serde_json::from_slice::<LoginErrors>(body)
        .map(|b| b.errors.join("; "))
        .unwrap_or_default();
    KeyServiceError::Upstream { status, detail }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_token_is_replaced_at_two_thirds_of_its_lease() {
        assert_eq!(renew_at(1_000, 3_600), 3_400);
        assert_eq!(renew_at(1_000, 0), u64::MAX);
        assert_eq!(renew_at(u64::MAX - 1, 3_600), u64::MAX);
    }

    #[test]
    fn a_login_is_needed_with_nothing_held_or_once_the_renewal_time_is_reached() {
        assert!(needs_login(10, None));
        assert!(!needs_login(10, Some(11)));
        assert!(needs_login(11, Some(11)));
        assert!(needs_login(12, Some(11)));
    }

    #[test]
    fn a_refused_login_keeps_openbaos_words_and_a_sealed_one_is_sealed() {
        assert!(matches!(login_failure(503, b""), KeyServiceError::Sealed));
        let refused = login_failure(403, br#"{"errors":["invalid audience (aud) claim"]}"#);
        assert!(
            matches!(&refused, KeyServiceError::Upstream { status: 403, detail } if detail.contains("audience")),
            "{refused:?}"
        );
    }

    #[test]
    fn the_login_url_is_under_the_auth_mount() {
        assert_eq!(
            login_url("http://bao:8200/", "kubernetes"),
            "http://bao:8200/v1/auth/kubernetes/login"
        );
    }

    #[test]
    fn a_token_never_reaches_debug() {
        let shown = format!("{:?}", OpenBaoAuth::Token("s.sekret".into()));
        assert!(!shown.contains("sekret"), "{shown}");
    }
}
