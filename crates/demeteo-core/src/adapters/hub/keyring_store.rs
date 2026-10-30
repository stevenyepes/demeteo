//! [`HubSecretStore`] over the OS keyring, through `keyring` v4's `v1` API.
//!
//! The store `keyring` picks is its own `cfg(target_os)` choice; this module
//! adds none. Only the Secret Service path is exercised on CI, and only by
//! reasoning: no test touches a real keyring, because CI has no Secret
//! Service. The macOS Keychain and Windows Credential Manager paths are
//! unverified by any gate here.

use keyring::{Entry, Error};

use crate::domain::hub_pairing::keyring_names::{keyring_backend_name, KEYRING_SERVICE};
use crate::domain::hub_pairing::KeyringStatus;
use crate::error::AppError;
use crate::ports::hub::HubSecretStore;

pub struct KeyringSecretStore;

/// Never the error's `Display` for a variant that carries stored bytes:
/// `BadEncoding` and `BadDataFormat` hold the secret they failed to read.
fn keyring_error(op: &str, account: &str, err: &Error) -> AppError {
    let detail = match err {
        Error::BadEncoding(_) | Error::BadDataFormat(..) => "the stored value is unreadable".into(),
        Error::Ambiguous(items) => format!("{} entries match", items.len()),
        other => other.to_string(),
    };
    AppError::internal(format!("keyring {op} of {account} failed: {detail}"))
}

fn entry(op: &str, account: &str) -> Result<Entry, AppError> {
    Entry::new(KEYRING_SERVICE, account).map_err(|e| keyring_error(op, account, &e))
}

impl HubSecretStore for KeyringSecretStore {
    fn status(&self) -> KeyringStatus {
        KeyringStatus {
            backend: keyring_backend_name(std::env::consts::OS),
            available: Entry::store_status().is_ok(),
            reason: None,
        }
    }

    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        match entry("read", account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error("read", account, &e)),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<(), AppError> {
        entry("write", account)?
            .set_password(value)
            .map_err(|e| keyring_error("write", account, &e))
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        match entry("delete", account)?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error("delete", account, &e)),
        }
    }
}
