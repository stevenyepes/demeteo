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

// ── correction re-ask ───────────────────────────────────────────────

/// A turn that ends without a verdict object is re-asked in the same session,
/// and the re-ask is the last thing the model reads before it answers. It
/// offers `environment` for "something this project is not configured to
/// run", which describes a review of an unconfigured project word for word —
/// a third source of that advice, after `NotConfigured`'s block and the
/// verdict contract, and the one that arrives after the review starter's
/// override rather than before it. The prompt test pins the first turn only.
///
/// The starter answers it the only way it can while the engine is off limits:
/// its gate step's instructions, still in the resumed session's context, say
/// in advance that the re-ask does not change a `NOTHING RAN` verdict. The
/// first half of this test is what makes that sentence necessary; once the
/// `domain/` fix under `docs/OPEN_QUESTIONS.md` §21a stops the engine offering
/// `environment` there, including here, it fails and the sentence can go.
#[test]
fn the_review_gate_step_pre_empts_the_correction_reasks_environment_advice() {
    assert!(
        correction_prompt("verdict").contains("Use `environment`"),
        "the re-ask no longer advises `environment`; the starter's pre-emption \
         of it is now dead prose"
    );

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows/code-review.json");
    let raw = std::fs::read_to_string(path).expect("the code-review starter ships in-tree");
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let verifier_json = doc["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["id"] == "s-validate-branch")
        .expect("the shipped starter carries its gate step, s-validate-branch")["verifier"]
        .clone();
    let cfg: VerifierConfig = serde_json::from_value(verifier_json).unwrap();

    assert!(
        cfg.instructions
            .split('\n')
            .flat_map(|line| line.split(". "))
            .any(|s| s.contains("asks again")
                && s.contains("\"pass\"")
                && s.contains("NOTHING RAN")),
        "no sentence in s-validate-branch's instructions tells the model that a \
         re-ask for the verdict alone still takes \"pass\" when NOTHING RAN"
    );
}
