// Tests extracted from `src/adapters/step_executor/artifacts/declared.rs`
// (mirrored-tests convention). `super` resolves to that module.

use super::{note_undelivered_artifacts, MissingArtifact};

fn missing(name: &str, detail: &str) -> MissingArtifact {
    MissingArtifact {
        name: name.to_string(),
        detail: detail.to_string(),
    }
}

#[test]
fn nothing_missing_leaves_the_reason_untouched() {
    // The common case, and the one that must stay byte-identical: this string
    // is retry feedback, and appending boilerplate to every verdict would
    // dilute the part the next agent has to act on.
    let reason = "criterion 3 not met: the debounce is missing";
    assert_eq!(note_undelivered_artifacts(reason, &[]), reason);
}

#[test]
fn a_missing_report_is_named_without_displacing_the_verdict() {
    let reason = "criterion 3 not met: the debounce is missing";
    let out = note_undelivered_artifacts(
        reason,
        &[missing(
            "validation-report",
            "artifacts/validation-report.md",
        )],
    );

    // The verdict leads. It is what the rework step decomposes into tickets;
    // the artifact note is context, not a replacement.
    assert!(out.starts_with(reason), "verdict must lead; got:\n{out}");
    assert!(out.contains("validation-report"));
    assert!(out.contains("artifacts/validation-report.md"));
}

#[test]
fn several_undelivered_artifacts_are_all_named() {
    let out = note_undelivered_artifacts(
        "rejected",
        &[
            missing("validation-report", "artifacts/validation-report.md"),
            missing("coverage", "artifacts/coverage.json"),
        ],
    );
    assert!(out.contains("validation-report"));
    assert!(out.contains("coverage"));
}
