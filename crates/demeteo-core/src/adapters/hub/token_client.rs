//! The Hub's `/token`, over the async reqwest client.
//!
//! The authorization code in the body is single-use: a second POST of it is
//! refused, and the first may already have been redeemed. So nothing here
//! retries, follows a redirect (which would carry the code to another URL),
//! or formats the request, the response body or reqwest's URL-bearing error
//! text into an error. What a refusal means is
//! [`crate::domain::hub_pairing::token_refusal`]'s to decide.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;

use crate::domain::hub_pairing::{token_refusal, HubPairingError, PairingAnswer};
use crate::ports::hub::HubTokenEndpoint;

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
