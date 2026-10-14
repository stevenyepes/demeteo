//! Keyring entry names for the Hub pairing.
//!
//! The OS keyring is per user, not per app identifier, so the dev build
//! (`com.stvcloud.demeteo.dev`) and the installed app see the same entries
//! while each has its own SQLite. A fixed account name would let a dev
//! pairing overwrite the installed app's device key, leaving its pins naming
//! a key it no longer holds. Every account is therefore scoped by the
//! database's install id.

use super::HubSecret;

/// The service every Demeteo keyring entry uses (`application/memory.rs`).
pub const KEYRING_SERVICE: &str = "demeteo";

pub fn hub_keyring_account(install_id: &str, secret: HubSecret) -> String {
    let purpose = match secret {
        HubSecret::DeviceKey => "hub_device_key",
        HubSecret::RefreshToken => "hub_refresh_token",
    };
    format!("{purpose}:{install_id}")
}

/// The store `keyring` v4 selects for `os` (a `std::env::consts::OS` value).
///
/// The `keyring` crate chooses its store by `cfg(target_os)` in its `v1.rs`
/// and its `v1` API cannot name the store it chose, so this mirrors that
/// selection as a plain string match, testable from any OS.
pub fn keyring_backend_name(os: &str) -> &'static str {
    match os {
        "macos" => "macOS Keychain",
        "windows" => "Windows Credential Manager",
        "ios" | "android" => "no supported keyring",
        _ => "Secret Service",
    }
}
