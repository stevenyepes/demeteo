// Tests extracted from `crates/demeteo-runner/src/run.rs` (mirrored-tests convention). `super` = that module.

use super::merge_project_settings;
use demeteo_core::adapters::step_executor::setup::fetch_default_settings;
use demeteo_core::domain::feature_origin::FeatureOrigin;
use demeteo_core::domain::ids::ProjectId;

#[test]
fn none_client_reproduces_detected_strategy() {
    // MC-D4: an old client (no settings) → detected strategy verbatim,
    // exactly the pre-multi-client behavior.
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "trunk".to_string();
    detected.test_command = Some("cargo test".to_string());
    let out = merge_project_settings(detected, None, ProjectId::from("p1".to_string()), None);
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

    let out = merge_project_settings(
        detected,
        Some(client),
        ProjectId::from("p2".to_string()),
        None,
    );
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
    let out = merge_project_settings(
        detected,
        Some(client),
        ProjectId::from("p3".to_string()),
        None,
    );
    assert_eq!(out.worktree_strategy.default_branch, "develop");

    // Detected blank AND client blank → "main" (never an empty branch).
    let mut client2 = fetch_default_settings();
    client2.worktree_strategy.default_branch = String::new();
    let mut detected2 = fetch_default_settings().worktree_strategy;
    detected2.default_branch = String::new();
    let out2 = merge_project_settings(
        detected2,
        Some(client2),
        ProjectId::from("p4".to_string()),
        None,
    );
    assert_eq!(out2.worktree_strategy.default_branch, "main");
}

/// Detection succeeding with empty output is the way a blank arrives with no
/// client to fall through to: `git rev-parse --abbrev-ref origin/HEAD` returning
/// `Ok("")` is not an error, so `detect_worktree_strategy` hands back a strategy
/// naming no branch. The row it lands in is what `create_feature_branch`,
/// `merge_base` and the squash all read.
#[test]
fn a_blank_detected_branch_with_no_client_still_names_one() {
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = String::new();
    let out = merge_project_settings(detected, None, ProjectId::from("p8".to_string()), None);
    assert_eq!(out.worktree_strategy.default_branch, "main");
}

// ── The base a run declared ──────────────────────────────────────────────────
//
// Why the run's base outranks every other claimant on a runner-side
// `default_branch` is on `merge_project_settings`.

#[test]
fn a_declared_base_beats_the_detected_default_branch() {
    let mut client = fetch_default_settings();
    client.worktree_strategy.default_branch = "main".to_string();
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();

    let out = merge_project_settings(
        detected,
        Some(client),
        ProjectId::from("p5".to_string()),
        FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        }
        .base_branch(None),
    );

    assert_eq!(
        out.worktree_strategy.default_branch, "release/2.0",
        "the detected default is a branch this run did not declare, and every \
         answer that falls back to this field would be about that one instead"
    );
}

#[test]
fn a_pull_request_run_measures_itself_against_the_branch_it_merges_into() {
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();
    let origin = FeatureOrigin::Ref {
        fetch_spec: "refs/pull/12/head".to_string(),
        label: "PR #12".to_string(),
    };

    let out = merge_project_settings(
        detected,
        None,
        ProjectId::from("p6".to_string()),
        origin.base_branch(Some("develop")),
    );

    assert_eq!(out.worktree_strategy.default_branch, "develop");
}

#[test]
fn a_blank_declared_base_leaves_the_detected_one_standing() {
    let mut detected = fetch_default_settings().worktree_strategy;
    detected.default_branch = "master".to_string();
    let out = merge_project_settings(
        detected,
        None,
        ProjectId::from("p7".to_string()),
        Some("   "),
    );
    assert_eq!(
        out.worktree_strategy.default_branch, "master",
        "the row never carries an empty branch name"
    );
}

// ── RunSpec wire format (effort) ────────────────────────────────────
//
// `RunSpec` is the contract between the desktop app and this runner. The
// spec's version-skew rule (AGENTS.md §9.1) only holds if `effort` is
// genuinely optional in both directions.

#[test]
fn a_spec_without_effort_deserializes_to_none() {
    // An older desktop app sends no `effort` key at all. The runner must
    // accept the spec and inherit (project default, else High) — not fail.
    let spec: demeteo_core::domain::run_spec::RunSpec = serde_json::from_value(serde_json::json!({
        "title": "t",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "/tmp/repo",
        "workflow_json": { "steps": [] },
        "agent_kind": "claude-code",
        "model": "sonnet",
    }))
    .expect("a pre-effort spec must still parse");
    assert_eq!(spec.effort, None);
}

#[test]
fn a_spec_with_effort_round_trips_through_the_wire() {
    let spec: demeteo_core::domain::run_spec::RunSpec = serde_json::from_value(serde_json::json!({
        "title": "t",
        "description": "d",
        "provider": { "kind": "github", "host": "github.com" },
        "repo_path": "/tmp/repo",
        "workflow_json": { "steps": [] },
        "agent_kind": "codex",
        "model": "gpt-5",
        "effort": "max",
    }))
    .expect("a spec carrying effort must parse");
    assert_eq!(
        spec.effort,
        Some(demeteo_core::domain::models::EffortLevel::Max)
    );

    let json = serde_json::to_string(&spec).expect("RunSpec serializes");
    let back: demeteo_core::domain::run_spec::RunSpec =
        serde_json::from_str(&json).expect("RunSpec round-trips");
    assert_eq!(back.effort, spec.effort);
    // The canonical spelling is the lowercase one, on the wire as in the DB.
    assert!(json.contains("\"effort\":\"max\""));
}
