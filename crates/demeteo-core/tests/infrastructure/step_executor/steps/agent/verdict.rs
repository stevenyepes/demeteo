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
        verdict_disposition(ParsedVerdict::Pass, &[], EnvironmentReading::Unjudgeable),
        VerdictDisposition::Pass
    ));
    assert!(
        matches!(
            verdict_disposition(
                ParsedVerdict::Pass,
                &[missing("report", "never written")],
                EnvironmentReading::Unjudgeable
            ),
            VerdictDisposition::Pass
        ),
        "the missing-deliverable check is the completion stage's, not the verdict's"
    );
}

// ── fail ────────────────────────────────────────────────────────────

#[test]
fn a_failing_verdict_with_everything_delivered_keeps_its_reason_verbatim() {
    match verdict_disposition(
        fail("criterion 3 is not met"),
        &[],
        EnvironmentReading::Unjudgeable,
    ) {
        VerdictDisposition::Fail(f) => assert_eq!(f.reason, "criterion 3 is not met"),
        _ => panic!("a fail verdict must be Fail"),
    }
}

#[test]
fn an_undelivered_report_is_appended_to_the_reason_not_substituted_for_it_s14() {
    match verdict_disposition(
        fail("criterion 3 is not met"),
        &[missing("review-report", "no artifact matched")],
        EnvironmentReading::Unjudgeable,
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
        EnvironmentReading::Unjudgeable,
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
        EnvironmentReading::Unjudgeable,
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
        EnvironmentReading::Unjudgeable,
    ) {
        VerdictDisposition::Unjudgeable { message, .. } => assert!(
            !message.contains("report"),
            "S14's note belongs on the retryable arm only"
        ),
        _ => panic!("expected Unjudgeable"),
    }
}

// ── evidence ────────────────────────────────────────────────────────

#[test]
fn an_evidence_verdict_parks_rather_than_opening_a_rework_loop() {
    let gap = EvidenceGap {
        reason: "no proof each test failed first".into(),
        criteria: vec!["AC6".into()],
    };
    match verdict_disposition(
        ParsedVerdict::Evidence(gap.clone()),
        &[],
        EnvironmentReading::Unjudgeable,
    ) {
        VerdictDisposition::Evidence(got) => assert_eq!(got, gap),
        _ => panic!("an evidence verdict must reach a human, not the rework loop"),
    }
}

/// The contract, the parser and the re-ask are one menu; a re-ask that drops
/// `evidence` pushes a verifier that had it right into `fail`.
#[test]
fn the_correction_reask_offers_evidence() {
    let prompt = correction_prompt("verdict");
    assert!(prompt.contains("\"verdict\": \"evidence\""), "{prompt}");
    assert!(prompt.contains("Use `evidence`"), "{prompt}");
}

// ── missing ─────────────────────────────────────────────────────────

#[test]
fn no_readable_verdict_terminates_with_the_infrastructure_prefix() {
    match verdict_disposition(
        ParsedVerdict::Missing("no JSON object found".into()),
        &[],
        EnvironmentReading::Unjudgeable,
    ) {
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
        EnvironmentReading::Unjudgeable,
    ) {
        VerdictDisposition::NoVerdict(message) => assert!(!message.contains("report")),
        _ => panic!("expected NoVerdict"),
    }
}

// ── nothing-ran policy ──────────────────────────────────────────────

#[test]
fn an_environment_verdict_is_a_pass_where_the_step_declares_nothing_to_configure() {
    assert!(matches!(
        verdict_disposition(
            ParsedVerdict::Environment("no gate is configured".into()),
            &[],
            EnvironmentReading::Pass,
        ),
        VerdictDisposition::Pass
    ));
}

/// The review starter's gate step reports what the gates said and judges
/// nothing the project must configure, so an absent harness is its `pass`.
/// Three engine texts advise `environment` for that case — the `NotConfigured`
/// block, the verdict contract and the correction re-ask — and the starter
/// used to argue with all three in prose, the re-ask arriving a turn after
/// the argument. The field is read where the verdict is read, so it covers
/// the re-ask by construction and the prose has nothing left to override.
#[test]
fn the_review_gate_step_declares_nothing_to_configure_in_the_field_not_in_prose() {
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

    assert_eq!(
        cfg.when_nothing_ran,
        crate::domain::verifier::NothingRanPolicy::Pass
    );
    for stale in ["does not apply", "asks again"] {
        assert!(
            !cfg.instructions.contains(stale),
            "the field carries the policy now; `{stale}` is prose arguing with an \
             engine text that no longer needs arguing with"
        );
    }
    assert!(
        correction_prompt("verdict").contains("Use `environment`"),
        "the re-ask still offers `environment` to every step; the field, not the \
         menu, is what makes it a pass here"
    );
}
