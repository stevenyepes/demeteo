//! The Hub's `POST /git-token`, over the async reqwest client (decision 71).
//!
//! Authenticated as the attachment fetch is, and with the same hygiene: the
//! access token goes out in the `Authorization` header and the minted token
//! comes back in the body, and neither reaches an error, a `tracing` field or
//! a redirect, which is why none is followed. Unlike an attachment, a token
//! can be asked for again, so a `401` earns one access-token refresh and one
//! second request.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::redirect::Policy;
use reqwest::StatusCode;

use crate::domain::hub_pairing::{GitTokenAnswer, HubPairingError, SecretString};
use crate::ports::hub::{GitTokenFetchError, HubGitToken, HubGitTokenFetcher};
use crate::shared::time::now_secs;

use super::device_key::DeviceKey;
use super::socket::AccessTokens;
use super::{random_token, Refresher};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A token answer is two short fields; anything longer is not one.
const MAX_ANSWER_BYTES: usize = 16 * 1024;

/// Holds the pairing's token state as [`AccessTokens`], never the
/// [`super::HubPairing`] itself, as the attachment fetcher does.
pub struct HubGitTokens {
    client: reqwest::Client,
    origin: String,
    key: DeviceKey,
    tokens: Arc<dyn AccessTokens>,
}

/// The URL of the route, which is also the `htu` of the proof that calls it.
fn git_token_url(origin: &str) -> String {
    format!("{origin}/git-token")
}

/// A category only: reqwest's own text names the URL.
fn unavailable(e: &reqwest::Error) -> GitTokenFetchError {
    let what = if e.is_timeout() {
        "the git token request timed out"
    } else if e.is_connect() {
        "could not connect to the Hub"
    } else {
        "the git token request failed"
    };
    GitTokenFetchError::Unavailable(what.into())
}

impl HubGitTokens {
    /// `origin` is the pinned one, as [`crate::domain::hub_pairing::hub_origin`]
    /// spells it, and `key` the device key the pairing bound its tokens to.
    pub fn new(
        origin: String,
        key: DeviceKey,
        tokens: Arc<dyn AccessTokens>,
    ) -> Result<Self, HubPairingError> {
        let client = reqwest::Client::builder()
            .user_agent("demeteo-orchestrator")
            .redirect(Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| HubPairingError::Transport("could not build the HTTP client".into()))?;
        Ok(Self {
            client,
            origin,
            key,
            tokens,
        })
    }

    async fn access_token(&self) -> Result<SecretString, GitTokenFetchError> {
        if let Some(token) = self.tokens.access_token() {
            return Ok(token);
        }
        self.refreshed().await
    }

    async fn refreshed(&self) -> Result<SecretString, GitTokenFetchError> {
        self.tokens.refresh_access().await.map_err(|_| {
            GitTokenFetchError::Unavailable("no Hub access token could be refreshed".into())
        })
    }

    async fn post(
        &self,
        url: &str,
        body: &str,
        access: &SecretString,
    ) -> Result<reqwest::Response, GitTokenFetchError> {
        let proof_failed = |_| GitTokenFetchError::Unavailable("could not sign a proof".into());
        let jti = random_token().map_err(proof_failed)?;
        let dpop = self
            .key
            .dpop_proof_for("POST", url, now_secs(), &jti, Some(access.expose()))
            .map_err(proof_failed)?;
        self.client
            .post(url)
            .header(AUTHORIZATION, format!("DPoP {}", access.expose()))
            .header("DPoP", dpop)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_owned())
            .send()
            .await
            .map_err(|e| unavailable(&e))
    }
}

async fn read_answer(mut response: reqwest::Response) -> Result<HubGitToken, GitTokenFetchError> {
    let unreadable = || GitTokenFetchError::Unavailable("the Hub sent an unreadable token".into());
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| unavailable(&e))? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_ANSWER_BYTES {
            return Err(unreadable());
        }
    }
    let answer: GitTokenAnswer = serde_json::from_slice(&bytes).map_err(|_| unreadable())?;
    let (token, expires_at) = answer.into_parts();
    Ok(HubGitToken { token, expires_at })
}

#[async_trait]
impl HubGitTokenFetcher for HubGitTokens {
    async fn fetch(
        &self,
        request_id: &str,
        feature_id: &str,
    ) -> Result<HubGitToken, GitTokenFetchError> {
        let url = git_token_url(&self.origin);
        let body =
            serde_json::json!({ "request_id": request_id, "feature_id": feature_id }).to_string();
        let access = self.access_token().await?;
        let mut response = self.post(&url, &body, &access).await?;
        if response.status() == StatusCode::UNAUTHORIZED {
            // Another caller may have refreshed since this token was read;
            // only a token nobody has replaced is worth a refresh of its own.
            let access = match self.tokens.access_token() {
                Some(newer) if newer.expose() != access.expose() => newer,
                _ => self.refreshed().await?,
            };
            response = self.post(&url, &body, &access).await?;
        }
        let status = response.status();
        if !status.is_success() {
            return Err(GitTokenFetchError::Refused {
                status: status.as_u16(),
            });
        }
        read_answer(response).await
    }
}

/// See [`super::HubPairing::git_tokens`].
pub struct PairedGitTokens(Refresher);

impl PairedGitTokens {
    pub(super) fn new(refresher: Refresher) -> Self {
        Self(refresher)
    }
}

#[async_trait]
impl HubGitTokenFetcher for PairedGitTokens {
    async fn fetch(
        &self,
        request_id: &str,
        feature_id: &str,
    ) -> Result<HubGitToken, GitTokenFetchError> {
        let fetcher = self
            .0
            .git_token_fetcher()
            .map_err(|_| GitTokenFetchError::Unavailable("the Hub pairing did not load".into()))?
            .ok_or_else(|| GitTokenFetchError::Unavailable("this runner is not paired".into()))?;
        fetcher.fetch(request_id, feature_id).await
    }
}
