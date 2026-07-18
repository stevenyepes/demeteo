// Tests extracted from `src-tauri/src/commands/remote_runner.rs` (mirrored-tests convention). `super` = that module.

use super::{declared_remote_paths, mime_for_path, shadow_step_artifacts_stale, stamp_client_id};
use crate::domain::ids::{FeatureId, StepExecutionId, StepId};
use crate::domain::models::feature::StepExecution;

/// Stand-in `StepExecution` builder mirroring the one in
/// `crates/demeteo-core/tests/infrastructure/step_executor/gate_redirect_reset.rs` —
/// `shadow_step_artifacts_stale` only reads `status`/`tokens`/`wall_clock_secs`/
/// `cost_usd`, but the struct has no `Default` so every field needs a
/// plausible value.
fn make_step(status: &str, tokens: i64, wall_clock_secs: u64, cost_usd: f64) -> StepExecution {
    StepExecution {
        id: StepExecutionId::from("se-1".to_string()),
        feature_id: FeatureId::from("f-1".to_string()),
        step_id: StepId::from("s-critic".to_string()),
        step_index: 5,
        step_kind: "agent".to_string(),
        status: status.to_string(),
        cost_usd: Some(cost_usd),
        tokens: Some(tokens),
        wall_clock_secs: Some(wall_clock_secs),
        artifact_path: Some("artifacts/critic-review.md".to_string()),
        artifact_paths: vec!["artifacts/critic-review.md".to_string()],
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

#[test]
fn shadow_stale_false_when_no_existing_shadow_yet() {
    // First-ever hydrate for this step: nothing to compare against, and
    // the artifact-count gate in `cache_step_artifacts` already handles
    // "first pull" correctly on its own.
    let fresh = make_step("completed", 86143, 147, 0.42);
    assert!(!shadow_step_artifacts_stale(None, &fresh));
}

#[test]
fn shadow_stale_false_when_nothing_changed() {
    // Same fields both times (e.g. re-hydrating an idle, already-cached
    // step): must not force a redundant re-pull on every poll tick.
    let existing = make_step("completed", 86143, 147, 0.42);
    let fresh = make_step("completed", 86143, 147, 0.42);
    assert!(!shadow_step_artifacts_stale(Some(&existing), &fresh));
}

#[test]
fn shadow_stale_true_when_tokens_changed_but_status_and_count_would_look_identical() {
    // This is the exact regression: a gate redirect re-runs `s-critic`,
    // it lands back on `completed` with the same single declared
    // artifact (`critic-review.md`) as before — an artifact-count check
    // alone sees no difference — but the token/cost/wall-clock numbers
    // the runner reports are fresh, proving a new attempt actually ran.
    let existing = make_step("completed", 86143, 147, 0.42);
    let fresh = make_step("completed", 137678, 95, 0.61);
    assert!(shadow_step_artifacts_stale(Some(&existing), &fresh));
}

#[test]
fn shadow_stale_true_when_status_differs() {
    let existing = make_step("completed", 100, 10, 0.1);
    let fresh = make_step("running", 100, 10, 0.1);
    assert!(shadow_step_artifacts_stale(Some(&existing), &fresh));
}

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
