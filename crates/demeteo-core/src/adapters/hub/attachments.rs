//! The Hub's `GET /attachments/{attachment_id}`, over the async reqwest
//! client (docs/HUB.md §6.3 step 4).
//!
//! The Hub deletes an attachment as it hands it over, so nothing here
//! retries, and a refresh is the only second request a fetch can make. The
//! Hub is not trusted with the content's integrity: what arrives is checked
//! against the size and SHA-256 of the request the Hub signed, and its bytes
//! are never read past that size. The access token goes out in the
//! `Authorization` header and nowhere else — not into an error, a `tracing`
//! field, or a redirect, which is why none is followed.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use demeteo_hub_protocol::AttachmentRef;
use reqwest::header::AUTHORIZATION;
use reqwest::redirect::Policy;
use sha2::{Digest, Sha256};

use crate::application::attachments::MAX_ATTACHMENT_BYTES;
use crate::domain::hub_pairing::{HubPairingError, SecretString};
use crate::ports::hub::{AttachmentFetchError, HubAttachmentFetcher};
use crate::shared::time::now_secs;

use super::device_key::DeviceKey;
use super::socket::AccessTokens;
use super::{random_token, Refresher};

/// Long enough for the Hub's default 25 MiB limit over a slow uplink.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

/// Holds the pairing's token state as [`AccessTokens`], never the
/// [`super::HubPairing`] itself, as the socket does.
pub struct HubAttachments {
    client: reqwest::Client,
    origin: String,
    key: DeviceKey,
    tokens: Arc<dyn AccessTokens>,
}

impl HubAttachments {
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

    async fn access_token(&self) -> Result<SecretString, AttachmentFetchError> {
        if let Some(token) = self.tokens.access_token() {
            return Ok(token);
        }
        self.tokens.refresh_access().await.map_err(|_| {
            AttachmentFetchError::Unavailable("no Hub access token could be refreshed".into())
        })
    }
}

/// The URL of attachment `id`, which is also the `htu` of the proof that
/// fetches it: `crates/demeteo-hub/src/attachments/fetch.rs` builds the one it
/// compares from its configured origin and the same path.
fn attachment_url(origin: &str, id: &str) -> String {
    format!("{origin}/attachments/{id}")
}

/// The Hub mints every attachment id as a UUID. Anything else is refused
/// before it is put into the path the access token is presented to.
fn is_attachment_id(id: &str) -> bool {
    id.len() == 36 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// `bytes`, if they are what `attachment` declares. Shared with the test
/// double so that both refuse the same content.
pub(super) fn checked(
    attachment: &AttachmentRef,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, AttachmentFetchError> {
    if bytes.len() as u64 != attachment.size
        || format!("{:x}", Sha256::digest(&bytes)) != attachment.sha256
    {
        return Err(AttachmentFetchError::Mismatch);
    }
    Ok(bytes)
}

/// A category only: reqwest's own text names the URL.
fn unavailable(e: &reqwest::Error) -> AttachmentFetchError {
    let what = if e.is_timeout() {
        "the attachment fetch timed out"
    } else if e.is_connect() {
        "could not connect to the Hub"
    } else {
        "the attachment fetch failed"
    };
    AttachmentFetchError::Unavailable(what.into())
}

#[async_trait]
impl HubAttachmentFetcher for HubAttachments {
    async fn fetch(&self, attachment: &AttachmentRef) -> Result<Vec<u8>, AttachmentFetchError> {
        // `HubRequests::start` refuses this before any fetch; held here too
        // because the size is what the body below is buffered up to.
        if !is_attachment_id(&attachment.id) || attachment.size > MAX_ATTACHMENT_BYTES {
            return Err(AttachmentFetchError::Mismatch);
        }
        let url = attachment_url(&self.origin, &attachment.id);
        let access = self.access_token().await?;
        let proof_failed = |_| AttachmentFetchError::Unavailable("could not sign a proof".into());
        let jti = random_token().map_err(proof_failed)?;
        let dpop = self
            .key
            .dpop_proof_for("GET", &url, now_secs(), &jti, Some(access.expose()))
            .map_err(proof_failed)?;
        let mut response = self
            .client
            .get(&url)
            .header(AUTHORIZATION, format!("DPoP {}", access.expose()))
            .header("DPoP", dpop)
            .send()
            .await
            .map_err(|e| unavailable(&e))?;
        let status = response.status();
        if !status.is_success() {
            return Err(AttachmentFetchError::Refused {
                status: status.as_u16(),
            });
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| unavailable(&e))? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > attachment.size {
                return Err(AttachmentFetchError::Mismatch);
            }
        }
        checked(attachment, bytes)
    }
}

/// See [`super::HubPairing::attachments`].
pub struct PairedAttachments(Refresher);

impl PairedAttachments {
    pub(super) fn new(refresher: Refresher) -> Self {
        Self(refresher)
    }
}

#[async_trait]
impl HubAttachmentFetcher for PairedAttachments {
    async fn fetch(&self, attachment: &AttachmentRef) -> Result<Vec<u8>, AttachmentFetchError> {
        let fetcher = self
            .0
            .attachment_fetcher()
            .map_err(|_| AttachmentFetchError::Unavailable("the Hub pairing did not load".into()))?
            .ok_or_else(|| {
                AttachmentFetchError::Unavailable("this desktop is not paired".into())
            })?;
        fetcher.fetch(attachment).await
    }
}
