//! What desktop pairing with the Demeteo Hub needs from the outside world:
//! the OS keyring, SQLite, the Hub's `/token` and `/socket`, and a browser.
//!
//! The split between the first two is the point. [`HubPinRepository`] holds
//! only the public [`HubPins`]; the device private key and the refresh token
//! go through [`HubSecretStore`] and nowhere else (AGENTS.md §2).

use async_trait::async_trait;
use demeteo_hub_protocol::Snapshot;

use crate::domain::hub_pairing::{
    HubPairingError, HubPins, KeyringStatus, PairingAnswer, RefreshAnswer, SecretString,
};
use crate::domain::hub_socket::RefreshEnd;
use crate::error::AppError;

/// The OS keyring, addressed by the account names in
/// [`crate::domain::hub_pairing::keyring_names`].
///
/// The stored refresh token has a **single writer**. The Hub rotates it on
/// every use and treats a replayed one as theft: two concurrent refreshes, or
/// a retry after an answer that was lost in transit, revoke the whole token
/// family and unpair this desktop. Whatever refreshes must serialize on one
/// owner and never retry a refresh it cannot prove went unanswered.
pub trait HubSecretStore: Send + Sync {
    fn status(&self) -> KeyringStatus;
    /// A missing entry is `Ok(None)`, not an error.
    fn get(&self, account: &str) -> Result<Option<String>, AppError>;
    fn set(&self, account: &str, value: &str) -> Result<(), AppError>;
    /// Deleting a missing entry is `Ok(())`.
    fn delete(&self, account: &str) -> Result<(), AppError>;
}

/// The `hub_install`, `hub_pairing` and `hub_pinned_passkeys` tables (V64).
pub trait HubPinRepository: Send + Sync {
    /// Creates the id on first call and returns the same one forever after,
    /// including across [`HubPinRepository::clear`].
    fn install_id(&self) -> Result<String, AppError>;
    fn load(&self) -> Result<Option<HubPins>, AppError>;
    /// Replaces the pairing and every pinned passkey in one transaction.
    fn save(&self, pins: &HubPins) -> Result<(), AppError>;
    /// Deletes the pairing and its passkeys in one transaction.
    fn clear(&self) -> Result<(), AppError>;
}

#[async_trait]
pub trait HubTokenEndpoint: Send + Sync {
    /// POST `form_body` (application/x-www-form-urlencoded) with exactly one
    /// DPoP header. The code it redeems is single-use, so this is never
    /// retried.
    async fn exchange(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<PairingAnswer, HubPairingError>;
}

/// The `refresh_token` grant on the same `/token`. A trait of its own rather
/// than a method of [`HubTokenEndpoint`], so a double of one is not obliged
/// to answer the other.
#[async_trait]
pub trait HubRefreshEndpoint: Send + Sync {
    /// POST `form_body` with exactly one DPoP header, **never retried**: see
    /// the single-writer note on [`HubSecretStore`]. The error says how far
    /// the request got, which is what decides whether the stored refresh
    /// token may be presented again.
    async fn refresh(
        &self,
        token_url: &str,
        form_body: String,
        dpop: String,
    ) -> Result<RefreshAnswer, RefreshEnd>;
}

/// What a dial of the Hub's `GET /socket` presents. `Debug` prints the
/// access token as `<redacted>`.
#[derive(Debug, Clone)]
pub struct SocketUpgrade {
    pub url: String,
    pub access_token: SecretString,
    /// A proof for `GET /socket` whose `ath` names `access_token`.
    pub dpop: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialFailure {
    /// The Hub answered the upgrade with a status other than `101`; `error`
    /// is the OAuth `error` of its body, when it had one.
    Refused { status: u16, error: Option<String> },
    /// No HTTP answer at all: DNS, TCP, TLS, or a timeout.
    Unreachable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketReceived {
    Text(String),
    /// A frame that proves the Hub is still there and carries nothing for
    /// the session: a ping, which the transport has already answered.
    Alive,
    /// The close code the Hub sent, or `None` when the connection ended
    /// without one.
    Closed(Option<u16>),
}

/// The connection is gone; the next [`HubSocketConnection::received`] says
/// how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketGone;

#[async_trait]
pub trait HubSocketDialer: Send + Sync {
    /// Upgrades with `Authorization: DPoP <access>` and exactly one `DPoP`
    /// header, the two the Hub's admission reads. Never retried here:
    /// whether to redial, and with which token, is
    /// [`crate::domain::hub_socket::after`]'s decision.
    async fn dial(
        &self,
        upgrade: SocketUpgrade,
    ) -> Result<Box<dyn HubSocketConnection>, DialFailure>;
}

#[async_trait]
pub trait HubSocketConnection: Send {
    /// Cancel-safe: dropping the future unresolved loses no frame, so it can
    /// race a wake-up or a deadline in a `select!`.
    async fn received(&mut self) -> SocketReceived;
    async fn send_text(&mut self, text: String) -> Result<(), SocketGone>;
    async fn close(self: Box<Self>, code: u16);
}

pub trait BrowserLauncher: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}

/// What this instance reports to the Hub, built from local storage and
/// nothing else. Synchronous because every read behind it is, so a caller on
/// the async runtime runs it inside `spawn_blocking`.
pub trait HubSnapshotSource: Send + Sync {
    fn snapshot(&self) -> Result<Snapshot, String>;
}
