// `super` = `domain::artifact_capture`. No doubles and no runtime — both
// renderings are pure over what the step did and did not deliver.

use super::*;

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

// ── missing_deliverables_message ────────────────────────────────────

#[test]
fn one_undelivered_artifact_is_singular() {
    let out = missing_deliverables_message(&[missing("plan", "artifacts/plan.md")]);
    assert!(
        out.contains("1 declared artifact was never produced"),
        "singular reads wrong in the plural: {out}"
    );
    assert!(!out.contains("declared artifacts were"));
}

#[test]
fn two_undelivered_artifacts_are_plural() {
    let out = missing_deliverables_message(&[
        missing("plan", "artifacts/plan.md"),
        missing("spec", "by name"),
    ]);
    assert!(
        out.contains("2 declared artifacts were never produced"),
        "plural reads wrong in the singular: {out}"
    );
    assert!(!out.contains("declared artifact was"));
}

#[test]
fn every_missing_name_and_detail_is_named() {
    let out = missing_deliverables_message(&[
        missing("plan", "artifacts/plan.md"),
        missing("spec", "by name"),
    ]);
    for token in ["'plan'", "artifacts/plan.md", "'spec'", "by name"] {
        assert!(out.contains(token), "missing `{token}` in: {out}");
    }
}

#[test]
fn the_message_names_the_causes_a_user_can_act_on() {
    let out = missing_deliverables_message(&[missing("plan", "artifacts/plan.md")]);
    for cause in ["failed", "different path", "model/config", "opencode.json"] {
        assert!(
            out.contains(cause),
            "`{cause}` is one of the four named causes of this failure class: {out}"
        );
    }
    assert!(out.contains("Nothing downstream can consume this step."));
}

// ── the two renderings are deliberately different ───────────────────

#[test]
fn the_two_renderings_stay_distinct() {
    let one = [missing("report", "artifacts/report.md")];
    let step_failure = missing_deliverables_message(&one);
    let verdict_note = note_undelivered_artifacts("rejected", &one);

    assert!(
        step_failure.starts_with("The step completed but"),
        "the step-failure wording is what the UI renders on the failed row"
    );
    assert!(
        verdict_note.starts_with("rejected"),
        "the verdict note is appended to feedback a downstream step reads"
    );
    assert_ne!(
        step_failure, verdict_note,
        "unifying the two is a behaviour change, not a cleanup"
    );
}
