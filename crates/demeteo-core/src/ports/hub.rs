//! What desktop pairing with the Demeteo Hub needs from the outside world:
//! the OS keyring, SQLite, the Hub's `/token`, and a browser.
//!
//! The split between the first two is the point. [`HubPinRepository`] holds
//! only the public [`HubPins`]; the device private key and the refresh token
//! go through [`HubSecretStore`] and nowhere else (AGENTS.md §2).

use async_trait::async_trait;

use crate::domain::hub_pairing::{HubPairingError, HubPins, KeyringStatus, PairingAnswer};
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

pub trait BrowserLauncher: Send + Sync {
    fn open(&self, url: &str) -> Result<(), String>;
}
