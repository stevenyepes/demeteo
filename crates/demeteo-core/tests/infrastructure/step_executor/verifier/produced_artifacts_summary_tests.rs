// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::format_produced_artifacts_summary;
use crate::domain::artifact::{Artifact, ArtifactSource};

#[test]
fn tool_write_artifact_points_at_its_worktree_path() {
    let arts = vec![Artifact::tool_write(
        "validation-report",
        "artifacts/validation-report.md",
        "Overall: READY TO SHIP".to_string(),
    )];
    let summary = format_produced_artifacts_summary(&arts);
    assert!(
        summary.contains("artifacts/validation-report.md"),
        "expected the worktree-relative path, got: {summary}"
    );
    assert!(
        summary.contains("Read"),
        "expected an instruction to Read the file, got: {summary}"
    );
}

#[test]
fn non_tool_write_artifact_falls_back_to_bare_name() {
    let arts = vec![Artifact {
        name: "code-diff".to_string(),
        mime: "text/x-diff".into(),
        content: "diff --git a/x b/x".to_string(),
        source: ArtifactSource::Diff {
            base: "abc123".to_string(),
            head: "WORKTREE".to_string(),
            path_filter: None,
        },
    }];
    let summary = format_produced_artifacts_summary(&arts);
    assert!(summary.contains("File/Artifact: code-diff"));
}

#[test]
fn empty_input_produces_empty_summary() {
    assert_eq!(format_produced_artifacts_summary(&[]), "");
}

#[test]
fn multiple_artifacts_each_get_their_own_line() {
    let arts = vec![
        Artifact::tool_write("validation-report", "artifacts/validation-report.md", "x"),
        Artifact::tool_write("critic-review", "artifacts/critic-review.md", "y"),
    ];
    let summary = format_produced_artifacts_summary(&arts);
    assert_eq!(summary.lines().count(), 2);
    assert!(summary.contains("artifacts/validation-report.md"));
    assert!(summary.contains("artifacts/critic-review.md"));
}
