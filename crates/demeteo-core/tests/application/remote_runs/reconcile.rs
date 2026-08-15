use super::{
    backfill_local_path, declared_remote_paths, mime_for_path, shadow_feature_patch,
    shadow_step_artifacts_stale,
};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaseline, HarnessBaselineRun};
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId};
use crate::domain::models::feature::{Feature, StepExecution};

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
    let fresh = make_step("completed", 86143, 147, 0.42);
    assert!(!shadow_step_artifacts_stale(None, &fresh));
}

#[test]
fn shadow_stale_false_when_nothing_changed() {
    let existing = make_step("completed", 86143, 147, 0.42);
    let fresh = make_step("completed", 86143, 147, 0.42);
    assert!(!shadow_step_artifacts_stale(Some(&existing), &fresh));
}

#[test]
fn shadow_stale_true_when_tokens_changed_but_status_and_count_would_look_identical() {
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
fn declared_paths_single_first_and_deduped() {
    let output = declared_remote_paths(
        Some("/w/report.md"),
        &["/w/report.md".to_string(), "/w/diff.patch".to_string()],
    );
    assert_eq!(output, vec!["/w/report.md", "/w/diff.patch"]);
}

#[test]
fn declared_paths_none_single_uses_list_only() {
    let output = declared_remote_paths(None, &["/w/a.txt".to_string(), "/w/b.txt".to_string()]);
    assert_eq!(output, vec!["/w/a.txt", "/w/b.txt"]);
}

#[test]
fn declared_paths_empty_when_nothing_declared() {
    assert!(declared_remote_paths(None, &[]).is_empty());
}

#[test]
fn backfill_finds_matching_local_file_by_stem() {
    let existing = vec!["/data/artifacts/f-1/se-1/critic-review.md".to_string()];
    let output = backfill_local_path(&existing, "/workspace/critic-review.md");
    assert_eq!(output, Some(existing[0].clone()));
}

#[test]
fn backfill_none_when_no_matching_local_file() {
    let existing = vec!["/data/artifacts/f-1/se-1/other-report.md".to_string()];
    assert_eq!(
        backfill_local_path(&existing, "/workspace/critic-review.md"),
        None
    );
}

#[test]
fn mime_inferred_from_extension() {
    assert_eq!(mime_for_path("/w/report.md"), "text/markdown");
    assert_eq!(mime_for_path("/w/change.diff"), "text/x-diff");
    assert_eq!(mime_for_path("/w/manifest.json"), "application/json");
    assert_eq!(mime_for_path("/w/LICENSE"), "text/plain");
}

// ── Shadow-feature replication (V37, decision 44) ────────────────────────────

fn runner_feature(harness_baseline: Option<HarnessBaseline>) -> Feature {
    Feature {
        id: FeatureId::from("f_shadow".to_string()),
        project_id: ProjectId::from("p_shadow".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "remote run".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 1.5,
        duration: "3m".to_string(),
        tokens: 42,
        created_at: 1_000,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: None,
        mr_state: Some("none".to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: Some("demeteo/features/f_shadow".to_string()),
    }
}

#[test]
fn the_shadow_patch_mirrors_the_measured_baseline() {
    // The update branch is the one that matters: the first poll inserts the
    // whole `Feature`, so a baseline missing from this patch is stale only
    // *after* the run measures it — silently, and only on a detached run.
    let baseline = HarnessBaseline {
        base_sha: "abc123".to_string(),
        harnesses: vec![HarnessBaselineRun {
            name: "unit".to_string(),
            command: "cargo test".to_string(),
            exit_ok: false,
            fingerprint: "fp".to_string(),
            output_ref: Some("/artifacts/unit.log".to_string()),
            environment: None,
            failing_tests: None,
            measured_at: 1_700,
            producer: BaselineProducer::Node,
        }],
    };
    let patch = shadow_feature_patch(&runner_feature(Some(baseline.clone())));
    assert_eq!(patch.harness_baseline, Some(Some(baseline)));
}

#[test]
fn the_shadow_patch_mirrors_the_branch_the_runner_cut_but_not_the_launch_inputs() {
    let patch = shadow_feature_patch(&runner_feature(None));
    assert_eq!(
        patch.resolved_branch,
        Some(Some("demeteo/features/f_shadow".to_string())),
        "only the runner knows what it named the branch"
    );
    assert!(patch.origin.is_none());
    assert!(patch.diff_base_branch.is_none());
}

#[test]
fn the_shadow_patch_mirrors_an_unmeasured_baseline_as_absent() {
    let patch = shadow_feature_patch(&runner_feature(None));
    assert_eq!(
        patch.harness_baseline,
        Some(None),
        "an unmeasured run must clear the shadow, not leave a stale record"
    );
}
