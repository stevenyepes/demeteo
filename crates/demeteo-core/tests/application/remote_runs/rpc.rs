use super::json_str;

#[test]
fn json_str_returns_some_for_object_key() {
    assert_eq!(
        json_str(&serde_json::json!({"status": "running"}), "status"),
        Some("running".to_string())
    );
}

#[test]
fn json_str_returns_none_for_missing_key() {
    assert_eq!(json_str(&serde_json::json!({}), "status"), None);
}

#[test]
fn json_str_returns_none_for_null_value() {
    assert_eq!(
        json_str(&serde_json::json!({"status": null}), "status"),
        None
    );
}

#[test]
fn json_str_returns_none_for_non_object_root() {
    assert_eq!(json_str(&serde_json::json!(["running"]), "status"), None);
}
