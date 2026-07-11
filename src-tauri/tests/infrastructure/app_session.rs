// Tests extracted from `src-tauri/src/commands/app_session.rs` (mirrored-tests convention). `super` = that module.

use super::*;

/// `get_app_info` must surface both `CARGO_PKG_VERSION` and the
/// compile-time `RELEASE_CHANNEL` constant.
#[test]
fn app_info_matches_constants() {
    let info = get_app_info();
    assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.channel, crate::RELEASE_CHANNEL);
}

/// Default channel is `"stable"` when `DEMETEO_RELEASE_CHANNEL` is not
/// set at compile time. Override the constant via env to verify the
/// fallback path; since the constant is baked at compile time, this
/// test simply ensures the chosen value is one of the two known
/// channels (a regression guard, not a true env-driven test).
#[test]
fn release_channel_is_a_known_value() {
    let c = crate::RELEASE_CHANNEL;
    assert!(
        c == "stable" || c == "nightly",
        "unexpected release channel: {c}"
    );
}

/// The serialized payload must use snake-case-friendly field names so
/// the TS wrapper can decode without manual renaming.
#[test]
fn app_info_serializes_field_names() {
    let info = AppInfo {
        version: "0.1.0".to_string(),
        channel: "nightly".to_string(),
    };
    let json = serde_json::to_value(&info).expect("serialize");
    assert_eq!(json["version"], "0.1.0");
    assert_eq!(json["channel"], "nightly");
    assert!(json.get("version").is_some());
    assert!(json.get("channel").is_some());
}
