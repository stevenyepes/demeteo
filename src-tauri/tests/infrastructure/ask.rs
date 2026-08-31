// Tests extracted from `src-tauri/src/commands/ask.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use serde_json::json;

/// The whole point of the double `Option` on `model`/`effort`: serde's derive
/// maps JSON `null` to `None`, which the repo's `?4 = patch.model.is_some()`
/// reads as "absent" and leaves the column alone. These three cases pin the
/// distinction the `AskThreadPatch` contract is built on.
#[test]
fn model_null_deserializes_to_an_explicit_clear() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({ "model": null })).unwrap();
    assert_eq!(patch.model, Some(None));
}

#[test]
fn absent_model_deserializes_to_leave_alone() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(patch.model, None);
}

#[test]
fn model_value_deserializes_to_a_set() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({ "model": "x" })).unwrap();
    assert_eq!(patch.model, Some(Some("x".to_string())));
}

#[test]
fn effort_null_deserializes_to_an_explicit_clear() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({ "effort": null })).unwrap();
    assert_eq!(patch.effort, Some(None));
}

#[test]
fn absent_effort_deserializes_to_leave_alone() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(patch.effort, None);
}

#[test]
fn effort_value_deserializes_to_a_set() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({ "effort": "xhigh" })).unwrap();
    assert_eq!(patch.effort, Some(Some(EffortLevel::XHigh)));
}

/// `network` is non-nullable by design — `COALESCE(?8, network)` reads a plain
/// `Option`, so a `null` here is a malformed patch rather than a clear.
#[test]
fn network_stays_a_single_option() {
    let patch: AskSettingsPatch = serde_json::from_value(json!({ "network": false })).unwrap();
    assert_eq!(patch.network, Some(false));
}
