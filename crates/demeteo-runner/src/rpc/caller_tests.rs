// Tests of `caller` in `crates/demeteo-runner/src/rpc/mod.rs`. `super` = that module.

use super::caller;
use crate::hub::HUB_CLIENT_ID;
use serde_json::json;

#[test]
fn a_socket_caller_claiming_the_hub_is_refused() {
    let error = caller(&json!({ "client_id": HUB_CLIENT_ID, "run_id": "hub-1" }))
        .expect_err("the Hub's owner id is reserved");
    assert!(error.contains("reserved"), "{error}");
}

#[test]
fn a_desktop_caller_and_a_legacy_one_pass_through() {
    assert_eq!(
        caller(&json!({ "client_id": "laptop-1" })).as_deref(),
        Ok("laptop-1")
    );
    assert_eq!(caller(&json!({})).as_deref(), Ok(""));
}
