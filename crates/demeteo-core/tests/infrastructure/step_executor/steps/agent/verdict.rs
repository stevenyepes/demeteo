// Tests for the agent step's verdict disposition. `super` = the `verdict`
// module. No doubles and no runtime — the decision is pure.

use super::*;

fn missing(name: &str, detail: &str) -> MissingArtifact {
    MissingArtifact {
        name: name.into(),
        detail: detail.into(),
    }
}

fn fail(reason: &str) -> ParsedVerdict {
    ParsedVerdict::Fail(VerdictFailure::from_reason(reason))
}

// ── pass ────────────────────────────────────────────────────────────

#[test]
fn a_pass_is_a_pass_whatever_went_undelivered() {
    assert!(matches!(
        verdict_disposition(ParsedVerdict::Pass, &[]),
        VerdictDisposition::Pass
    ));
    assert!(
        matches!(
            verdict_disposition(ParsedVerdict::Pass, &[missing("report", "never written")]),
            VerdictDisposition::Pass
        ),
        "the missing-deliverable check is the completion stage's, not the verdict's"
    );
}

// ── fail ────────────────────────────────────────────────────────────

#[test]
fn a_failing_verdict_with_everything_delivered_keeps_its_reason_verbatim() {
    match verdict_disposition(fail("criterion 3 is not met"), &[]) {
        VerdictDisposition::Fail(f) => assert_eq!(f.reason, "criterion 3 is not met"),
        _ => panic!("a fail verdict must be Fail"),
    }
}

#[test]
fn an_undelivered_report_is_appended_to_the_reason_not_substituted_for_it_s14() {
    match verdict_disposition(
        fail("criterion 3 is not met"),
        &[missing("review-report", "no artifact matched")],
    ) {
        VerdictDisposition::Fail(f) => {
            assert!(
                f.reason.starts_with("criterion 3 is not met"),
                "the verdict is the more actionable outcome and must lead: {}",
                f.reason
            );
            assert!(
                f.reason.contains("review-report"),
                "the step downstream attaches the report by name and will find nothing"
            );
            assert!(f.reason.contains("no artifact matched"));
        }
        _ => panic!("a fail verdict must stay a Fail even with nothing delivered"),
    }
}

#[test]
fn every_undelivered_deliverable_is_named() {
    match verdict_disposition(
        fail("rejected"),
        &[missing("spec", "no match"), missing("plan", "wrong path")],
    ) {
        VerdictDisposition::Fail(f) => {
            for token in ["spec", "no match", "plan", "wrong path"] {
                assert!(
                    f.reason.contains(token),
                    "missing `{token}` in: {}",
                    f.reason
                );
            }
        }
        _ => panic!("expected Fail"),
    }
}

// ── environment ─────────────────────────────────────────────────────

#[test]
fn an_environment_verdict_terminates_with_the_configuration_prefix() {
    match verdict_disposition(
        ParsedVerdict::Environment("no build_command is set".into()),
        &[],
    ) {
        VerdictDisposition::Unjudgeable { reason, message } => {
            assert_eq!(reason, "no build_command is set");
            assert_eq!(
                message,
                "[project configuration — retrying cannot fix this] no build_command is set"
            );
        }
        _ => panic!("an environment verdict must not open a rework loop"),
    }
}

#[test]
fn an_environment_verdict_ignores_undelivered_artifacts() {
    match verdict_disposition(
        ParsedVerdict::Environment("no build_command is set".into()),
        &[missing("report", "never written")],
    ) {
        VerdictDisposition::Unjudgeable { message, .. } => assert!(
            !message.contains("report"),
            "S14's note belongs on the retryable arm only"
        ),
        _ => panic!("expected Unjudgeable"),
    }
}

// ── missing ─────────────────────────────────────────────────────────

#[test]
fn no_readable_verdict_terminates_with_the_infrastructure_prefix() {
    match verdict_disposition(ParsedVerdict::Missing("no JSON object found".into()), &[]) {
        VerdictDisposition::NoVerdict(message) => assert_eq!(
            message,
            "[verifier infrastructure error — no usable verdict from the validate turn] \
             no JSON object found"
        ),
        _ => panic!("an unreadable verdict is not a rejection of the work"),
    }
}

#[test]
fn no_readable_verdict_ignores_undelivered_artifacts() {
    match verdict_disposition(
        ParsedVerdict::Missing("no JSON object found".into()),
        &[missing("report", "never written")],
    ) {
        VerdictDisposition::NoVerdict(message) => assert!(!message.contains("report")),
        _ => panic!("expected NoVerdict"),
    }
}
