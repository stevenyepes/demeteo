// Tests for `steps/finalize/context.rs` (mirrored-tests convention).

use super::*;

/// Demeteo's own bookkeeping commits describe the machinery, not the work.
/// The whole point of the squash is to make them disappear, so they are also
/// worthless as input to the summary — and actively misleading, since an
/// agent shown "chore: merge subtask sub-2" tends to write about merging.
#[test]
fn plumbing_commits_are_recognised_as_demeteos_own() {
    assert!(is_plumbing_commit("chore: merge subtask sub-2", "f-123"));
    assert!(is_plumbing_commit(
        "chore: resolve merge conflicts with feature/f-123",
        "f-123"
    ));
    assert!(is_plumbing_commit(
        "chore: resolve sync conflicts with origin/main",
        "f-123"
    ));
    assert!(is_plumbing_commit(
        "feat(f-123): implement the thing",
        "f-123"
    ));
}

#[test]
fn real_work_commits_are_kept() {
    assert!(!is_plumbing_commit("feat(api): add retry budget", "f-123"));
    assert!(!is_plumbing_commit("fix: handle the empty case", "f-123"));
    // Another feature's step commit is not ours to filter, and shouldn't
    // appear on this branch anyway.
    assert!(!is_plumbing_commit("feat(f-999): other work", "f-123"));
    // A human's genuine chore commit must survive.
    assert!(!is_plumbing_commit("chore: bump deps", "f-123"));
}

fn a_step(step_id: &str, artifact_paths: Vec<String>) -> crate::domain::models::StepExecution {
    use crate::domain::ids::{FeatureId, StepExecutionId, StepId};
    crate::domain::models::StepExecution {
        id: StepExecutionId::new(format!("se-{step_id}")),
        feature_id: FeatureId::new("f-1"),
        step_id: StepId::new(step_id),
        step_index: 0,
        step_kind: "agent".to_string(),
        status: "completed".to_string(),
        cost_usd: None,
        tokens: None,
        wall_clock_secs: None,
        artifact_path: None,
        artifact_paths,
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        last_failure_fingerprint: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// The gatherer inlines prose reports, skips the raw diff (already inlined
/// separately), skips the finalize step itself, and tolerates a missing file.
#[test]
fn gather_prior_artifacts_is_selective_and_best_effort() {
    let dir = std::env::temp_dir().join(format!("demeteo-finalize-ctx-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let spec = dir.join("implementation-spec.md");
    std::fs::write(&spec, "the intended approach").unwrap();
    let code_diff = dir.join("code-diff.diff");
    std::fs::write(&code_diff, "RAW DIFF CONTENT").unwrap();
    let missing = dir.join("was-never-captured.md");

    let store = crate::adapters::artifact_store::fs::FsArtifactStore::new(dir.clone());
    let steps = vec![
        a_step("s-spec", vec![spec.to_string_lossy().into_owned()]),
        a_step(
            "s-implement",
            vec![code_diff.to_string_lossy().into_owned()],
        ),
        a_step("s-validate", vec![missing.to_string_lossy().into_owned()]),
        a_step("s-finalize", vec![spec.to_string_lossy().into_owned()]),
    ];

    let out = gather_prior_artifacts(&store, &steps, "s-finalize");

    assert!(
        out.contains("the intended approach"),
        "prose report is inlined"
    );
    assert!(
        out.contains("implementation-spec.md"),
        "labelled by filename"
    );
    assert!(out.contains("s-spec"), "labelled by step id");
    assert!(
        !out.contains("RAW DIFF CONTENT"),
        ".diff artifact is skipped"
    );
    // s-finalize's own artifact (same spec path) is not double-counted as a
    // separate labelled block for the finalize step.
    assert_eq!(out.matches("from step `s-finalize`").count(), 0);

    // No report-producing steps at all → empty, so the prompt degrades to
    // diff-only rather than erroring.
    assert!(gather_prior_artifacts(&store, &[], "s-finalize").is_empty());

    let _ = std::fs::remove_dir_all(&dir);
}
