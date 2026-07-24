use super::stamp_client_id;

#[test]
fn stamp_client_id_injects_and_preserves_keys() {
    let params = serde_json::json!({ "run_id": "laptop-1", "spec": { "title": "x" } });
    let output = stamp_client_id(params, "client-A");
    assert_eq!(output["client_id"], "client-A");
    assert_eq!(output["run_id"], "laptop-1");
    assert_eq!(output["spec"]["title"], "x");
}

#[test]
fn stamp_client_id_leaves_non_object_untouched() {
    let output = stamp_client_id(serde_json::json!("bare"), "client-A");
    assert_eq!(output, serde_json::json!("bare"));
}

#[test]
fn stamp_client_id_preserves_array_root_payload() {
    let payload = serde_json::json!([{"run_id": "laptop-1"}, "bare"]);
    let output = stamp_client_id(payload.clone(), "client-A");
    assert_eq!(output, payload);
}
