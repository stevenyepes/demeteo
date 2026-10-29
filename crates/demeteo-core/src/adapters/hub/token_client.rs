//! The Hub's `/token`, over the async reqwest client.
//!
//! Both grants spend what their body carries: an authorization code is
//! single-use, and a refresh token is rotated by the first POST that reaches
//! the Hub and revokes its family on the second. So nothing here retries,
//! follows a redirect (which would carry the code or token to another URL),
//! or formats the request, the response body or reqwest's URL-bearing error
//! text into an error. What a refusal means is
//! [`crate::domain::hub_pairing::token_refusal`]'s to decide, and for a
//! refresh [`crate::domain::hub_socket::refresh_refusal`]'s.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;

use crate::domain::hub_pairing::{token_refusal, HubPairingError, PairingAnswer, RefreshAnswer};
use crate::domain::hub_socket::{refresh_refusal, RefreshEnd};
use crate::ports::hub::{HubRefreshEndpoint, HubTokenEndpoint};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ReqwestTokenEndpoint {
    client: reqwest::Client,
}

impl ReqwestTokenEndpoint {
    pub fn new() -> Result<Self, HubPairingError> {
        let client = reqwest::Client::builder()
            .user_agent("demeteo-orchestrator")
            .redirect(Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| HubPairingError::Transport("could not build the HTTP client".into()))?;
        Ok(Self { client })
    }
}

/// A category only: reqwest's own text names the URL.
fn transport_error(e: &reqwest::Error) -> HubPairingError {
    let what = if e.is_timeout() {
        "the request to /token timed out"
    } else if e.is_connect() {
        "could not connect to /token"
    } else {
        "the request to /token failed"
    };
    HubPairingError::Transport(what.into())
}

#[async_trait]
impl HubTokenEndpoint for ReqwestTokenEndpoint {
    async fn exchange(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, HubPairingError> {
        let response = self
            .client
            .post(token_url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("DPoP", dpop)
            .body(form_body)
            .send()
            .await
            .map_err(|e| transport_error(&e))?;
        let success = response.status().is_success();
        let body = response.bytes().await.map_err(|e| transport_error(&e))?;
        if !success {
            return Err(token_refusal(&body));
        }
        // serde's message can quote a value from the body, a token included.
        serde_json::from_slice(&body).map_err(|_| {
            HubPairingError::MalformedAnswer("the /token answer is not a pairing answer".into())
        })
    }
}

/// How far a failed refresh got. reqwest reports a builder or connect error
/// before any byte of the request is written; after that, the Hub may have
/// read it and rotated the token, whatever became of the answer.
fn refresh_failure(e: &reqwest::Error) -> RefreshEnd {
    if e.is_builder() || e.is_connect() {
        RefreshEnd::Unsent
    } else {
        RefreshEnd::Unanswered
    }
}

#[async_trait]
impl HubRefreshEndpoint for ReqwestTokenEndpoint {
    async fn refresh(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<RefreshAnswer, RefreshEnd> {
        let response = self
            .client
            .post(token_url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("DPoP", dpop)
            .body(form_body)
            .send()
            .await
            .map_err(|e| refresh_failure(&e))?;
        let status = response.status();
        let body = response.bytes().await.map_err(|_| RefreshEnd::Unanswered)?;
        if !status.is_success() {
            return Err(refresh_refusal(status.as_u16(), &body));
        }
        // A `200` whose tokens cannot be read is as lost as no answer: the
        // Hub has rotated, and the old refresh token is spent.
        serde_json::from_slice(&body).map_err(|_| RefreshEnd::Unanswered)
    }
}
