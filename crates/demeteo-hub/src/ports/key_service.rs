use async_trait::async_trait;
use std::fmt;

/// A fresh data key and the same key wrapped under a named Transit key.
///
/// Only `wrapped` may be persisted (`attachments.wrapped_key`); `plaintext`
/// exists to encrypt one payload and is then dropped. `Debug` omits it.
pub struct DataKey {
    pub plaintext: Vec<u8>,
    pub wrapped: String,
}

impl fmt::Debug for DataKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataKey")
            .field("plaintext", &"<redacted>")
            .field("wrapped", &self.wrapped)
            .finish()
    }
}

/// `Sealed` is its own variant because unseal is a manual operator step: a
/// caller must be able to tell "wait for the operator" from "this request is
/// wrong" without parsing a message.
#[derive(Debug, thiserror::Error)]
pub enum KeyServiceError {
    #[error("key service is sealed")]
    Sealed,
    #[error("unknown key `{0}`")]
    UnknownKey(String),
    /// Refused before any request was made: the name could not be addressed
    /// safely, so it was never sent.
    #[error("invalid key name `{0}`")]
    InvalidKeyName(String),
    #[error("invalid key service mount `{0}`")]
    InvalidMount(String),
    #[error("key service unreachable: {0}")]
    Transport(String),
    #[error("key service answered HTTP {status}: {detail}")]
    Upstream { status: u16, detail: String },
    #[error("key service returned a malformed response: {0}")]
    Malformed(String),
}

/// Envelope encryption as the Hub needs it. Deliberately has no persistence:
/// callers store what comes back, and `docs/HUB.md` §9 keeps plaintext out of
/// Postgres.
#[async_trait]
pub trait KeyService: Send + Sync {
    /// Returns the ciphertext in the service's own text form (`vault:v1:…`).
    async fn encrypt(&self, key: &str, plaintext: &[u8]) -> Result<String, KeyServiceError>;

    async fn decrypt(&self, key: &str, ciphertext: &str) -> Result<Vec<u8>, KeyServiceError>;

    async fn data_key(&self, key: &str) -> Result<DataKey, KeyServiceError>;
}
