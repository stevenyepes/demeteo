//! Tauri command that exposes the runtime application version + release
//! channel on the **About** tab of the preferences screen.
//!
//! The version is read from [`tauri::AppHandle::package_info`], which is
//! the single source of truth — it picks up both the manifest version
//! (`Cargo.toml`) and any Tauri config override (`.release/tauri-version-
//! override.json`), so CI nightly builds correctly show `0.1.0-32` while
//! tagged stable releases show `0.1.0`.
//!
//! Channel is derived from the version string + build profile so we don't
//! have to coordinate a separate build-time env var:
//!   * version contains a `-` pre-release suffix → `nightly`
//!   * else `cfg!(debug_assertions)`              → `nightly`
//!   * else                                       → `stable`

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppVersion {
    pub version: String,
    pub channel: String,
}

#[tauri::command]
pub fn get_app_version(app: tauri::AppHandle) -> AppVersion {
    let version = app.package_info().version.to_string();
    let channel = derive_channel(&version).to_string();
    AppVersion { version, channel }
}

fn derive_channel(version: &str) -> &'static str {
    if version.contains('-') || cfg!(debug_assertions) {
        "nightly"
    } else {
        "stable"
    }
}
