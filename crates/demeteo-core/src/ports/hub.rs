//! What desktop pairing with the Demeteo Hub, and the requests it relays,
//! need from the outside world: the OS keyring, SQLite, the Hub's `/token`,
//! `/socket` and `/attachments`, and a browser.
//!
//! The split between the first two is the point. [`HubPinRepository`] holds
//! only the public [`HubPins`]; the device private key and the refresh token
//! go through [`HubSecretStore`] and nowhere else (AGENTS.md §2).

use async_trait::async_trait;
use demeteo_hub_protocol::{AttachmentRef, RequestResult, Snapshot};

use crate::domain::hub_pairing::{
    HubPairingError, HubPins, KeyringStatus, PairingAnswer, RefreshAnswer, SecretString,
};
use crate::domain::hub_request::passkeys::{TrustWrite, TrustedPasskey};
use crate::domain::hub_request::{Consumption, HubLocalPolicy, HubRequestLogEntry};
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

/// The trusted passkey set in `hub_pinned_passkeys`, with the signature
/// counter each one last had accepted (V66, docs/HUB.md §8). A trait of its
/// own rather than methods of [`HubPinRepository`], so a double of that is
/// not obliged to answer these.
pub trait HubPasskeyTrust: Send + Sync {
    /// In pairing order.
    fn trusted(&self) -> Result<Vec<TrustedPasskey>, AppError>;
    /// A compare-and-set: `Ok(false)`, with nothing written, when the row is
    /// gone or its count is no longer `expected`. Never read-then-write: two
    /// assertions racing between a `SELECT` and an `UPDATE` would both read
    /// the old count and both be accepted.
    fn advance_count(&self, credential_id: &str, expected: u32, new: u32)
        -> Result<bool, AppError>;
    /// The signer's [`HubPasskeyTrust::advance_count`] and the new row, in one
    /// transaction that commits both or neither. `public_key` is stored as
    /// given: the endorsement challenge commits to those exact bytes. An id
    /// already pinned is `AlreadyPinned`, never a replaced key. No request
    /// path reaches this while `decide` refuses the kind (docs/HUB.md §11
    /// item 20).
    fn endorse(
        &self,
        signer: &str,
        expected: u32,
        new: u32,
        credential_id: &str,
        public_key: &str,
        pinned_at: i64,
    ) -> Result<TrustWrite, AppError>;
    /// The signer's [`HubPasskeyTrust::advance_count`] and the deletion of
    /// `target`, in one transaction that commits both or neither. The signer's
    /// row must still exist and survives the deletion, so revocations by
    /// distinct signers cannot empty the set however they interleave. The
    /// store also refuses to delete the last row whatever the caller checked.
    /// That is defence in depth against a self-revocation reaching the store,
    /// the only write it can catch, which `revocation_verdict` refuses first
    /// as `self_revocation`.
    fn revoke(
        &self,
        signer: &str,
        expected: u32,
        new: u32,
        target: &str,
    ) -> Result<TrustWrite, AppError>;
}

/// The `hub_consumed_requests` and `hub_request_log` tables (V65).
pub trait HubRequestRepository: Send + Sync {
    /// Claims the id in one `INSERT`; a row already there is
    /// `AlreadyConsumed`. Never check-then-insert: two deliveries of one id
    /// racing between a `SELECT` and an `INSERT` would both read as fresh.
    fn try_consume(
        &self,
        request_id: &str,
        expires_at_secs: u64,
        now_secs: u64,
    ) -> Result<Consumption, AppError>;
    /// Forgets only ids that expired longer ago than the clock skew between
    /// this desktop and the Hub could explain, so a replay the Hub still
    /// delivers finds its row.
    fn prune_consumed(&self, now_secs: u64) -> Result<(), AppError>;
    fn append_log(&self, entry: &HubRequestLogEntry) -> Result<(), AppError>;
    /// Newest first.
    fn list_log(&self, limit: usize) -> Result<Vec<HubRequestLogEntry>, AppError>;
}

/// The `hub_local_policy` row (V65), read side only. This is what the request
/// handler is given; [`HubPolicyWriter`] is a trait of its own so that no
/// request path can hold a way to widen what the Hub may do (docs/HUB.md §7).
pub trait HubPolicyReader: Send + Sync {
    /// A missing row is [`HubLocalPolicy::default`]: everything off, both
    /// ceilings unset.
    fn load(&self) -> Result<HubLocalPolicy, AppError>;
}

/// Written only from the desktop's Settings. Never put one on
/// [`crate::state::AppContext`]: the request handler is built over it.
pub trait HubPolicyWriter: Send + Sync {
    fn save(&self, policy: &HubLocalPolicy) -> Result<(), AppError>;
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

/// Why [`HubAttachmentFetcher::fetch`] returned no bytes. No variant
/// carries a token, a URL or the content.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttachmentFetchError {
    /// What arrived is not what the signed request declared: its byte count
    /// is not `size`, or its lowercase-hex SHA-256 is not `sha256`. Also an
    /// `id` that is not a UUID, which no Hub attachment has. Answered
    /// `RefusalReason::AttachmentMismatch`.
    #[error("the attachment does not match the size and SHA-256 its request declared")]
    Mismatch,
    /// The Hub answered with a non-2xx status. A `404` is also what a second
    /// fetch of one attachment gets: the Hub deletes it on the first.
    #[error("the Hub refused the attachment fetch with HTTP {status}")]
    Refused { status: u16 },
    /// No answer, or no access token or device key to ask with.
    #[error("the attachment could not be fetched: {0}")]
    Unavailable(String),
}

/// The Hub's `GET /attachments/{id}`, which hands an attachment over once
/// and deletes it (docs/HUB.md §6.3). So nothing calls this twice for one
/// attachment, and nothing calls it before every check that could refuse the
/// request has passed: a fetched attachment is gone whatever follows.
#[async_trait]
pub trait HubAttachmentFetcher: Send + Sync {
    /// The content, only once its size and SHA-256 are the ones `attachment`
    /// declares.
    async fn fetch(&self, attachment: &AttachmentRef) -> Result<Vec<u8>, AttachmentFetchError>;
}

/// What the instance socket hands each request frame to. A port rather than
/// [`crate::application::hub_requests::HubRequests`] itself so the socket
/// adapter does not depend on `application/`.
///
/// It does not break the ownership cycle: the bound handler holds the
/// [`crate::state::AppContext`], whose `hub` holds the
/// [`crate::adapters::hub::HubPairing`] whose slot holds the handler. Both are
/// built once and live as long as the process, so the cycle is a known
/// process-lifetime hold, not a leak that grows.
#[async_trait]
pub trait HubRequestHandler: Send + Sync {
    /// The `result` to send for `text`, or `None` when nothing may be sent: a
    /// request whose signature does not verify, one whose judging could not
    /// read what it needs from storage, left unspent so its redelivery is
    /// judged afresh, or a frame with no `request_id` to answer.
    async fn handle(&self, text: &str) -> Option<RequestResult>;
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
