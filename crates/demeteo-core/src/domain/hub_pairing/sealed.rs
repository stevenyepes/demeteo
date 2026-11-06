//! The key a `demeteo-runner` seals its Hub secrets under: the one exception
//! to the keyring-only rule of AGENTS.md §2, for runner hosts only
//! (decision 64 in docs/DECISIONS.md).
//! `systemd-creds` sealed it and the unit loads it with
//! `LoadCredentialEncrypted=`, so the process finds it in
//! `$CREDENTIALS_DIRECTORY`. A host where it is missing does not pair.

use std::ffi::OsStr;
use std::path::PathBuf;

/// The credential's name, in the unit and in `$CREDENTIALS_DIRECTORY`.
pub const STORE_KEY_CREDENTIAL: &str = "hub-store-key";

/// The `backend` a sealed store reports, as the keyring reports its own.
pub const SEALED_BACKEND: &str = "systemd-creds";

pub const STORE_KEY_LEN: usize = 32;

/// Where the key is, given `$CREDENTIALS_DIRECTORY`; an unset or empty one
/// means the unit loaded no credential.
pub fn store_key_path(credentials_directory: Option<&OsStr>) -> Result<PathBuf, &'static str> {
    match credentials_directory {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir).join(STORE_KEY_CREDENTIAL)),
        _ => Err("CREDENTIALS_DIRECTORY is not set: the unit loads no hub-store-key credential"),
    }
}

/// An AES-256 key is exactly 32 bytes. Anything else is refused rather than
/// hashed or padded into one, which would hide a credential that is not the
/// one `demeteo-runner hub seal` made.
pub fn store_key(bytes: &[u8]) -> Result<[u8; STORE_KEY_LEN], &'static str> {
    <[u8; STORE_KEY_LEN]>::try_from(bytes).map_err(|_| "hub-store-key is not exactly 32 bytes")
}
