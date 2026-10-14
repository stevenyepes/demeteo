//! An in-memory [`HubSecretStore`] for tests, which never touch the real OS
//! keyring (CI has no Secret Service).
//!
//! It refuses every account but the two [`hub_keyring_account`] derives for
//! its install id, so a secret written under any other name fails the test
//! instead of landing quietly in the map.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use crate::domain::hub_pairing::keyring_names::hub_keyring_account;
use crate::domain::hub_pairing::{HubSecret, KeyringStatus};
use crate::error::AppError;
use crate::ports::hub::HubSecretStore;

pub struct MemorySecretStore {
    accounts: [String; 2],
    available: AtomicBool,
    entries: Mutex<BTreeMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new(install_id: &str) -> Self {
        Self {
            accounts: [
                hub_keyring_account(install_id, HubSecret::DeviceKey),
                hub_keyring_account(install_id, HubSecret::RefreshToken),
            ],
            available: AtomicBool::new(true),
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    /// An unavailable store also refuses every read and write, as a keyring
    /// with no backend does.
    pub fn set_available(&self, available: bool) {
        self.available.store(available, Ordering::SeqCst);
    }

    pub fn entries(&self) -> BTreeMap<String, String> {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<String, String>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn check(&self, account: &str) -> Result<(), AppError> {
        if !self.available.load(Ordering::SeqCst) {
            return Err(AppError::internal("memory secret store is unavailable"));
        }
        if !self.accounts.iter().any(|a| a == account) {
            return Err(AppError::validation(format!(
                "{account} is not a Hub keyring account of this install"
            )));
        }
        Ok(())
    }
}

impl HubSecretStore for MemorySecretStore {
    fn status(&self) -> KeyringStatus {
        KeyringStatus {
            backend: "in-memory test store",
            available: self.available.load(Ordering::SeqCst),
        }
    }

    fn get(&self, account: &str) -> Result<Option<String>, AppError> {
        self.check(account)?;
        Ok(self.lock().get(account).cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<(), AppError> {
        self.check(account)?;
        self.lock().insert(account.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), AppError> {
        self.check(account)?;
        self.lock().remove(account);
        Ok(())
    }
}
