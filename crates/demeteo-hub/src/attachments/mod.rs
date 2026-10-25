//! Attachments: a file the user attached to a Feature in the browser, held by
//! the Hub only until the instance it was staged for fetches it. The lifecycle
//! is `docs/HUB.md` §6.3; this records the rules the code here keeps to it.
//!
//! # The custody rule
//!
//! A stored row holds ciphertext and a Transit-wrapped data key, and nothing
//! else from which the content could be recovered. The upload has Transit mint
//! a fresh data key, seals the content and its digest under it through
//! [`envelope`], and drops the plaintext key before it returns. A fetch is the
//! only other place that key exists, unwrapped for that one response.
//!
//! # The fetch-once rule
//!
//! A fetch that answered with the content has already deleted the row, so a
//! second fetch of the same id is refused as if it had never existed. Nothing
//! of an attachment outlives its delivery.
//!
//! # The TTL rule
//!
//! Every row carries an `expires_at`. A staged attachment's is
//! [`AttachmentSettings::ttl_secs`] past its upload; binding it to a request
//! moves it to the request's own expiry. A sweep every
//! [`AttachmentSettings::sweep_secs`] deletes whatever has passed it, fetched
//! or not. A request whose outcome is recorded does not wait for it: its
//! attachments are deleted in the transaction that records the outcome.
//!
//! Dependencies run one way: this module uses `store`, and neither `store` nor
//! `socket` imports it.

pub mod envelope;
pub mod fetch;
pub mod port;
pub mod postgres;
pub mod sweep;

/// The most attachments one dispatch may name. Kept equal by hand to
/// `MAX_ATTACHMENTS_PER_FEATURE` in `crates/demeteo-core/src/application/attachments.rs`,
/// which the instance enforces: importing it would make the Hub depend on
/// `demeteo-core`, which `src-tauri/tests/hub_layout.rs` forbids.
pub const MAX_ATTACHMENTS_PER_DISPATCH: usize = 10;

#[derive(Debug, Clone, PartialEq)]
pub struct AttachmentSettings {
    /// The largest upload accepted, in plaintext bytes.
    pub max_bytes: u64,
    /// How long a staged attachment lives before a dispatch binds it.
    pub ttl_secs: u64,
    pub sweep_secs: u64,
}

impl Default for AttachmentSettings {
    fn default() -> Self {
        Self {
            max_bytes: 25 * 1024 * 1024,
            ttl_secs: 3600,
            sweep_secs: 60,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_documented_ones() {
        assert_eq!(
            AttachmentSettings::default(),
            AttachmentSettings {
                max_bytes: 26_214_400,
                ttl_secs: 3600,
                sweep_secs: 60,
            }
        );
        assert_eq!(MAX_ATTACHMENTS_PER_DISPATCH, 10);
    }
}
