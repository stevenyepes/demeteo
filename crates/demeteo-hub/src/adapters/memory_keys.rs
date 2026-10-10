use crate::ports::{DataKey, KeyService, KeyServiceError};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

const PREFIX: &str = "memory";
const DATA_KEY_LEN: usize = 32;

/// Test double: reversible and not cryptographic. Errors on every key name it
/// was not constructed with, on all three methods, so a test that reaches for
/// the wrong key fails instead of passing against a default answer. Never
/// wired from `Config` or `main.rs`.
pub struct MemoryKeys {
    keys: BTreeSet<String>,
    counter: AtomicU64,
}

impl MemoryKeys {
    pub fn with_keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            keys: keys.into_iter().map(Into::into).collect(),
            counter: AtomicU64::new(0),
        }
    }

    fn require(&self, key: &str) -> Result<(), KeyServiceError> {
        if self.keys.contains(key) {
            Ok(())
        } else {
            Err(KeyServiceError::UnknownKey(key.to_owned()))
        }
    }

    fn seal(key: &str, plaintext: &[u8]) -> String {
        format!("{PREFIX}:{key}:{}", STANDARD.encode(plaintext))
    }

    fn unseal(key: &str, ciphertext: &str) -> Result<Vec<u8>, KeyServiceError> {
        let body = ciphertext
            .strip_prefix(PREFIX)
            .and_then(|rest| rest.strip_prefix(':'))
            .and_then(|rest| rest.strip_prefix(key))
            .and_then(|rest| rest.strip_prefix(':'))
            .ok_or_else(|| KeyServiceError::Malformed("not ciphertext of this key".into()))?;
        STANDARD
            .decode(body)
            .map_err(|e| KeyServiceError::Malformed(e.to_string()))
    }
}

#[async_trait]
impl KeyService for MemoryKeys {
    async fn encrypt(&self, key: &str, plaintext: &[u8]) -> Result<String, KeyServiceError> {
        self.require(key)?;
        Ok(Self::seal(key, plaintext))
    }

    async fn decrypt(&self, key: &str, ciphertext: &str) -> Result<Vec<u8>, KeyServiceError> {
        self.require(key)?;
        Self::unseal(key, ciphertext)
    }

    async fn data_key(&self, key: &str) -> Result<DataKey, KeyServiceError> {
        self.require(key)?;
        let n = self.counter.fetch_add(1, Ordering::Relaxed);
        let plaintext: Vec<u8> = (0..DATA_KEY_LEN)
            .map(|i| (n as u8).wrapping_add(i as u8))
            .collect();
        let wrapped = Self::seal(key, &plaintext);
        Ok(DataKey { plaintext, wrapped })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KNOWN: &str = "known";
    const OTHER: &str = "other";

    fn double() -> MemoryKeys {
        MemoryKeys::with_keys([KNOWN])
    }

    fn is_unknown<T>(r: Result<T, KeyServiceError>) -> bool {
        matches!(r, Err(KeyServiceError::UnknownKey(name)) if name == OTHER)
    }

    #[tokio::test]
    async fn unknown_key_errors_on_all_three_methods() {
        let keys = double();
        let wrapped = keys.encrypt(KNOWN, b"x").await.unwrap();

        assert!(is_unknown(keys.encrypt(OTHER, b"x").await));
        assert!(is_unknown(keys.decrypt(OTHER, &wrapped).await));
        assert!(is_unknown(keys.data_key(OTHER).await));
    }

    #[tokio::test]
    async fn configured_key_round_trips_including_empty_and_non_utf8() {
        let keys = double();
        for plaintext in [&b""[..], &[0xff, 0xfe, 0x00, 0x80][..], &b"hello"[..]] {
            let ct = keys.encrypt(KNOWN, plaintext).await.unwrap();
            assert_eq!(keys.decrypt(KNOWN, &ct).await.unwrap(), plaintext);
        }
    }

    #[tokio::test]
    async fn data_key_wrapped_form_decrypts_to_its_plaintext() {
        let keys = double();
        let dk = keys.data_key(KNOWN).await.unwrap();
        assert_eq!(
            keys.decrypt(KNOWN, &dk.wrapped).await.unwrap(),
            dk.plaintext
        );
        assert_ne!(dk.plaintext, keys.data_key(KNOWN).await.unwrap().plaintext);
    }

    #[tokio::test]
    async fn ciphertext_of_another_key_is_rejected() {
        let keys = MemoryKeys::with_keys([KNOWN, OTHER]);
        let ct = keys.encrypt(KNOWN, b"x").await.unwrap();
        assert!(keys.decrypt(OTHER, &ct).await.is_err());
    }
}
