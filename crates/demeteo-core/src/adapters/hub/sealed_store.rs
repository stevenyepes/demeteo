//! [`HubSecretStore`] for a host with no keyring: each value AES-256-GCM
//! sealed in SQLite under a key the process is handed at start. The rule that
//! allows it, and confines it to `demeteo-runner` hosts, is in
//! [`crate::domain::hub_pairing::sealed`].
//!
//! The account name is the associated data, so a sealed value moved to
//! another account's row fails to open rather than answering for it. A fresh
//! random nonce per write keeps a rotation of the refresh token from reusing
//! one under the same key.
//!
//! Without a key the store is unavailable and says why, which pairing
//! refuses with before any network call. Nothing here formats a plaintext,
//! a key or a ciphertext into an error.

use std::ffi::OsStr;
use std::io::Read;
use std::sync::Arc;

use aws_lc_rs::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};

use crate::domain::hub_pairing::sealed::{store_key, store_key_path, STORE_KEY_LEN};
use crate::domain::hub_pairing::KeyringStatus;
use crate::error::AppError;
use crate::ports::hub::{HubSealedSecretRepository, HubSecretStore, SealedSecret};

pub struct SealedSecretStore {
    repo: Arc<dyn HubSealedSecretRepository>,
    key: Result<LessSafeKey, &'static str>,
    backend: &'static str,
}

impl SealedSecretStore {
    /// `key` is what [`load_store_key`] answered: the key, or why there is
    /// none.
    pub fn new(
        repo: Arc<dyn HubSealedSecretRepository>,
        key: Result<[u8; STORE_KEY_LEN], &'static str>,
        backend: &'static str,
    ) -> Self {
        let key = key.and_then(|bytes| {
            UnboundKey::new(&AES_256_GCM, &bytes)
                .map(LessSafeKey::new)
                .map_err(|_| "hub-store-key is not an AES-256 key")
        });
        Self { repo, key, backend }
    }

    fn key(&self) -> Result<&LessSafeKey, AppError> {
        self.key.as_ref().map_err(|reason| {
            AppError::internal(format!(
                "the {} store is unavailable: {reason}",
                self.backend
            ))
        })
    }
}

impl HubSecretStore for SealedSecretStore {
    fn status(&self) -> KeyringStatus {
        KeyringStatus {
            backend: self.backend,
            available: self.key.is_ok(),
            reason: self.key.as_ref().err().copied(),
        }
    }

    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        let key = self.key()?;
        let Some(sealed) = self.repo.load_sealed(account)? else {
            return Ok(None);
        };
        let unreadable = || AppError::internal(format!("the sealed {account} does not open"));
        let nonce = <[u8; NONCE_LEN]>::try_from(sealed.nonce.as_slice())
            .map(Nonce::assume_unique_for_key)
            .map_err(|_| unreadable())?;
        let mut in_out = sealed.ciphertext;
        let plain = key
            .open_in_place(nonce, Aad::from(account.as_bytes()), &mut in_out)
            .map_err(|_| unreadable())?;
        String::from_utf8(plain.to_vec())
            .map(Some)
            .map_err(|_| unreadable())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), AppError> {
        let key = self.key()?;
        let mut nonce = [0u8; NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|e| AppError::internal(format!("no OS randomness for a nonce: {e}")))?;
        let mut in_out = value.as_bytes().to_vec();
        key.seal_in_place_append_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(account.as_bytes()),
            &mut in_out,
        )
        .map_err(|_| AppError::internal(format!("could not seal {account}")))?;
        self.repo.store_sealed(
            account,
            &SealedSecret {
                nonce: nonce.to_vec(),
                ciphertext: in_out,
            },
        )
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        self.key()?;
        self.repo.delete_sealed(account)
    }
}

/// A fresh store key from the OS, for `demeteo-runner hub seal` to hand
/// `systemd-creds` without writing it anywhere.
pub fn generate_store_key() -> Result<[u8; STORE_KEY_LEN], String> {
    let mut key = [0u8; STORE_KEY_LEN];
    getrandom::fill(&mut key).map_err(|e| format!("no OS randomness for a store key: {e}"))?;
    Ok(key)
}

/// The store key a unit loaded with `LoadCredentialEncrypted=`, given the
/// value of `$CREDENTIALS_DIRECTORY`; or why there is none, for
/// [`SealedSecretStore::new`].
pub fn load_store_key(
    credentials_directory: Option<&OsStr>,
) -> Result<[u8; STORE_KEY_LEN], &'static str> {
    let path = store_key_path(credentials_directory)?;
    let file = std::fs::File::open(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => "no hub-store-key credential was loaded",
        _ => "hub-store-key could not be opened",
    })?;
    let mut bytes = Vec::with_capacity(STORE_KEY_LEN + 1);
    file.take(STORE_KEY_LEN as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "hub-store-key could not be read")?;
    store_key(&bytes)
}
