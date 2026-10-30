//! The Hub's `/token` and `/device_authorization`, over the async reqwest
//! client.
//!
//! Every grant spends what its body carries: an authorization code is
//! single-use, a refresh token is rotated by the first POST that reaches
//! the Hub and revokes its family on the second, and an approved device code
//! is spent by the answer that carries the pairing. So nothing here retries,
//! follows a redirect (which would carry the code or token to another URL),
//! or formats the request, the response body or reqwest's URL-bearing error
//! text into an error. What a refusal means is
//! [`crate::domain::hub_pairing::token_refusal`]'s to decide, and for a
//! refresh [`crate::domain::hub_socket::refresh_refusal`]'s, and for a
//! device-code poll [`crate::domain::hub_pairing::device::device_grant_refusal`]'s.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::CONTENT_TYPE;
use reqwest::redirect::Policy;

use crate::domain::hub_pairing::device::{
    device_grant_refusal, DeviceAuthorization, DeviceGrantEnd,
};
use crate::domain::hub_pairing::{token_refusal, HubPairingError, PairingAnswer, RefreshAnswer};
use crate::domain::hub_socket::{refresh_refusal, RefreshEnd};
use crate::ports::hub::{HubDeviceEndpoint, HubRefreshEndpoint, HubTokenEndpoint};

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
    transport_error_at(e, "/token")
}

fn transport_error_at(e: &reqwest::Error, path: &str) -> HubPairingError {
    let what = if e.is_timeout() {
        format!("the request to {path} timed out")
    } else if e.is_connect() {
        format!("could not connect to {path}")
    } else {
        format!("the request to {path} failed")
    };
    HubPairingError::Transport(what)
}

impl ReqwestTokenEndpoint {
    fn form_post(&self, url: &str, form_body: String, dpop: String) -> reqwest::RequestBuilder {
        self.client
            .post(url)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("DPoP", dpop)
            .body(form_body)
    }
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
            .form_post(token_url, form_body, dpop)
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
            .form_post(token_url, form_body, dpop)
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

#[async_trait]
impl HubDeviceEndpoint for ReqwestTokenEndpoint {
    async fn authorize_device(
        &self,
        url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<DeviceAuthorization, HubPairingError> {
        let at = |e: &reqwest::Error| transport_error_at(e, "/device_authorization");
        let response = self
            .form_post(url, form_body, dpop)
            .send()
            .await
            .map_err(|e| at(&e))?;
        let success = response.status().is_success();
        let body = response.bytes().await.map_err(|e| at(&e))?;
        if !success {
            return Err(token_refusal(&body));
        }
        serde_json::from_slice(&body).map_err(|_| {
            HubPairingError::MalformedAnswer(
                "the /device_authorization answer is not a device authorization".into(),
            )
        })
    }

    async fn poll_device(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, DeviceGrantEnd> {
        let response = self
            .form_post(token_url, form_body, dpop)
            .send()
            .await
            .map_err(|_| DeviceGrantEnd::Unanswered)?;
        let success = response.status().is_success();
        let body = response
            .bytes()
            .await
            .map_err(|_| DeviceGrantEnd::Unanswered)?;
        if !success {
            return Err(device_grant_refusal(&body));
        }
        // The `200` spent the device code, so an unreadable one ends the
        // pairing rather than being polled for again.
        serde_json::from_slice(&body).map_err(|_| {
            DeviceGrantEnd::Refused(HubPairingError::MalformedAnswer(
                "the /token answer is not a pairing answer".into(),
            ))
        })
    }
}
