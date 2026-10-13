use crate::ports::{DataKey, KeyService, KeyServiceError};
use async_trait::async_trait;
use aws_lc_rs::error::Unspecified;
use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

const PREFIX: &str = "memory";
const DATA_KEY_LEN: usize = 32;

/// Test double. Its encryption is reversible and not cryptographic; its
/// signing is real, one Ed25519 key pair per name, so a test can verify a
/// signature under the public key a response carried. Errors on every key
/// name it was not constructed with, on every method, so a test that reaches
/// for the wrong key fails instead of passing against a default answer. Never
/// wired from `Config` or `main.rs`.
///
/// Compiled outside `#[cfg(test)]` under the `test-support` feature, and
/// panics where [`auth::testing`](crate::auth::testing) does, for the reason
/// given there.
pub struct MemoryKeys {
    keys: BTreeMap<String, Ed25519KeyPair>,
    sealed: bool,
    counter: AtomicU64,
}

impl MemoryKeys {
    pub fn with_keys<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            keys: keys
                .into_iter()
                .map(|name| (name.into(), generated_key_pair()))
                .collect(),
            sealed: false,
            counter: AtomicU64::new(0),
        }
    }

    /// A key service nobody has unsealed: every call answers `Sealed`, whatever
    /// name it carries, because a sealed service cannot say which keys exist.
    pub fn sealed() -> Self {
        Self {
            keys: BTreeMap::new(),
            sealed: true,
            counter: AtomicU64::new(0),
        }
    }

    fn require(&self, key: &str) -> Result<&Ed25519KeyPair, KeyServiceError> {
        if self.sealed {
            return Err(KeyServiceError::Sealed);
        }
        self.keys
            .get(key)
            .ok_or_else(|| KeyServiceError::UnknownKey(key.to_owned()))
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

    async fn verifying_key(&self, key: &str) -> Result<[u8; 32], KeyServiceError> {
        fixed(self.require(key)?.public_key().as_ref())
    }

    async fn sign(&self, key: &str, message: &[u8]) -> Result<[u8; 64], KeyServiceError> {
        fixed(self.require(key)?.sign(message).as_ref())
    }
}

fn generated_key_pair() -> Ed25519KeyPair {
    match Ed25519KeyPair::generate() {
        Ok(key_pair) => key_pair,
        Err(Unspecified) => panic!("memory keys: could not generate an Ed25519 key pair"),
    }
}

fn fixed<const N: usize>(bytes: &[u8]) -> Result<[u8; N], KeyServiceError> {
    bytes.try_into().map_err(|_| {
        KeyServiceError::Malformed(format!("{} bytes where {N} were due", bytes.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_lc_rs::signature::{UnparsedPublicKey, ED25519};

    const KNOWN: &str = "known";
    const OTHER: &str = "other";

    fn double() -> MemoryKeys {
        MemoryKeys::with_keys([KNOWN])
    }

    fn is_unknown<T>(r: Result<T, KeyServiceError>) -> bool {
        matches!(r, Err(KeyServiceError::UnknownKey(name)) if name == OTHER)
    }

    fn is_sealed<T>(r: Result<T, KeyServiceError>) -> bool {
        matches!(r, Err(KeyServiceError::Sealed))
    }

    fn verifies(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(message, signature)
            .is_ok()
    }

    #[tokio::test]
    async fn unknown_key_errors_on_every_method() {
        let keys = double();
        let wrapped = keys.encrypt(KNOWN, b"x").await.unwrap();

        assert!(is_unknown(keys.encrypt(OTHER, b"x").await));
        assert!(is_unknown(keys.decrypt(OTHER, &wrapped).await));
        assert!(is_unknown(keys.data_key(OTHER).await));
        assert!(is_unknown(keys.verifying_key(OTHER).await));
        assert!(is_unknown(keys.sign(OTHER, b"x").await));
    }

    #[tokio::test]
    async fn a_signature_verifies_under_the_verifying_key_of_the_same_name_only() {
        let keys = MemoryKeys::with_keys([KNOWN, OTHER]);
        let message = b"POST /v1/requests";
        let signature = keys.sign(KNOWN, message).await.unwrap();
        let own = keys.verifying_key(KNOWN).await.unwrap();
        let other = keys.verifying_key(OTHER).await.unwrap();

        assert!(verifies(&own, message, &signature));
        assert!(!verifies(&other, message, &signature));
        assert!(!verifies(&own, b"POST /v1/other", &signature));
        assert_eq!(own, keys.verifying_key(KNOWN).await.unwrap());
        assert_ne!(own, other);
    }

    #[tokio::test]
    async fn two_doubles_do_not_share_a_signing_key() {
        assert_ne!(
            double().verifying_key(KNOWN).await.unwrap(),
            double().verifying_key(KNOWN).await.unwrap()
        );
    }

    #[tokio::test]
    async fn the_sealed_double_answers_sealed_on_every_method() {
        let keys = MemoryKeys::sealed();

        assert!(is_sealed(keys.encrypt(KNOWN, b"x").await));
        assert!(is_sealed(keys.decrypt(KNOWN, "memory:known:eA==").await));
        assert!(is_sealed(keys.data_key(KNOWN).await));
        assert!(is_sealed(keys.verifying_key(KNOWN).await));
        assert!(is_sealed(keys.sign(KNOWN, b"x").await));
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
