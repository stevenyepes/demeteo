// Tests extracted from `crates/demeteo-runner/src/run.rs` (mirrored-tests convention). `super` = that module.

use super::merge_project_settings;
use demeteo_core::adapters::step_executor::setup::fetch_default_settings;
use demeteo_core::domain::ids::ProjectId;

#[test]
fn none_client_reproduces_detected_strategy() {
    // MC-D4: an old client (no settings) → detected strategy verbatim,
    // exactly the pre-multi-client behavior.
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "trunk".to_string();
    detected.test_command = Some("cargo test".to_string());
    let out = merge_project_settings(detected, None, ProjectId::from("p1".to_string()));
    assert_eq!(out.project_id.as_str(), "p1");
    assert_eq!(out.worktree_strategy.default_branch, "trunk");
    assert_eq!(
        out.worktree_strategy.test_command.as_deref(),
        Some("cargo test")
    );
}

#[test]
fn client_tunables_win_but_detected_default_branch_wins() {
    let mut client = fetch_default_settings();
    client.worktree_strategy.default_branch = "stale-main".to_string();
    client.worktree_strategy.test_command = Some("npm test".to_string());
    client.worktree_strategy.prepare_command = Some("npm ci".to_string());
    client.worktree_strategy.extra_writable_paths = vec!["node_modules/".to_string()];
    client.feature_lifecycle = "manual".to_string();

    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();
    detected.test_command = Some("SHOULD NOT WIN".to_string());

    let out = merge_project_settings(detected, Some(client), ProjectId::from("p2".to_string()));
    // Detected default_branch (read from origin/HEAD) wins over the
    // client's stale copy…
    assert_eq!(out.worktree_strategy.default_branch, "master");
    // …but the client wins on every other tunable.
    assert_eq!(
        out.worktree_strategy.test_command.as_deref(),
        Some("npm test")
    );
    assert_eq!(
        out.worktree_strategy.prepare_command.as_deref(),
        Some("npm ci")
    );
    assert_eq!(
        out.worktree_strategy.extra_writable_paths,
        vec!["node_modules/".to_string()]
    );
    assert_eq!(out.feature_lifecycle, "manual");
    assert_eq!(out.project_id.as_str(), "p2");
}

#[test]
fn empty_detected_branch_falls_back_to_client_then_main() {
    // Detected blank, client has a value → keep the client's.
    let mut client = fetch_default_settings();
    client.worktree_strategy.default_branch = "develop".to_string();
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "   ".to_string();
    let out = merge_project_settings(detected, Some(client), ProjectId::from("p3".to_string()));
    assert_eq!(out.worktree_strategy.default_branch, "develop");

    // Detected blank AND client blank → "main" (never an empty branch).
    let mut client2 = fetch_default_settings();
    client2.worktree_strategy.default_branch = String::new();
    let mut detected2 = fetch_default_settings().worktree_strategy;
    detected2.default_branch = String::new();
    let out2 = merge_project_settings(detected2, Some(client2), ProjectId::from("p4".to_string()));
    assert_eq!(out2.worktree_strategy.default_branch, "main");
}
