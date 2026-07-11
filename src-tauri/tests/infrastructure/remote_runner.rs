// Tests extracted from `src-tauri/src/commands/remote_runner.rs` (mirrored-tests convention). `super` = that module.

use super::{declared_remote_paths, mime_for_path, stamp_client_id};

#[test]
fn stamp_client_id_injects_and_preserves_keys() {
    // MC-D3: the single stamping site adds `client_id` without
    // disturbing the existing params a remote RPC already carries.
    let params = serde_json::json!({ "run_id": "laptop-1", "spec": { "title": "x" } });
    let out = stamp_client_id(params, "client-A");
    assert_eq!(out["client_id"], "client-A");
    assert_eq!(out["run_id"], "laptop-1");
    // Nested/other keys are untouched.
    assert_eq!(out["spec"]["title"], "x");
}

#[test]
fn stamp_client_id_leaves_non_object_untouched() {
    // A non-object payload can't carry a keyed id — return it verbatim
    // rather than corrupt it; the runner treats the caller as legacy.
    let out = stamp_client_id(serde_json::json!("bare"), "client-A");
    assert_eq!(out, serde_json::json!("bare"));
}

#[test]
fn declared_paths_single_first_and_deduped() {
    let out = declared_remote_paths(
        Some("/w/report.md"),
        &["/w/report.md".to_string(), "/w/diff.patch".to_string()],
    );
    // The legacy single path leads, and it is not repeated even though
    // it also appears in the list.
    assert_eq!(out, vec!["/w/report.md", "/w/diff.patch"]);
}

#[test]
fn declared_paths_none_single_uses_list_only() {
    let out = declared_remote_paths(None, &["/w/a.txt".to_string(), "/w/b.txt".to_string()]);
    assert_eq!(out, vec!["/w/a.txt", "/w/b.txt"]);
}

#[test]
fn declared_paths_empty_when_nothing_declared() {
    assert!(declared_remote_paths(None, &[]).is_empty());
}

#[test]
fn mime_inferred_from_extension() {
    assert_eq!(mime_for_path("/w/report.md"), "text/markdown");
    assert_eq!(mime_for_path("/w/change.diff"), "text/x-diff");
    assert_eq!(mime_for_path("/w/manifest.json"), "application/json");
    // Unknown / extensionless falls back to plain text.
    assert_eq!(mime_for_path("/w/LICENSE"), "text/plain");
}
