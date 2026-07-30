// Tests extracted from `src/adapters/step_executor/driver/verifier.rs`, moved
// with the code to `src/domain/harness_triage.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::*;

// ── classifier parsing, fail-safe to Regression ─────────────────────────

#[test]
fn parses_environment_verdict() {
    let raw = r#"{"category":"environment","reason":"gdk-3.0 dev package missing","remediation":"install libgtk-3-dev"}"#;
    match parse_triage_text(raw) {
        TriageVerdict::Environment {
            reason,
            remediation,
        } => {
            assert!(reason.contains("gdk-3.0"));
            assert_eq!(remediation, "install libgtk-3-dev");
        }
        _ => panic!("expected environment"),
    }
}

#[test]
fn parses_regression_verdict() {
    let raw = r#"prose... {"category":"regression","reason":"broken test","remediation":""}"#;
    assert_eq!(parse_triage_text(raw), TriageVerdict::Regression);
}

#[test]
fn environment_verdict_amid_prose_and_think_tags() {
    let raw = "<think>maybe env?</think>My verdict:\n{ \"category\": \"environment\", \"reason\": \"no compiler\", \"remediation\": \"install rustc\" }";
    assert!(matches!(
        parse_triage_text(raw),
        TriageVerdict::Environment { .. }
    ));
}

#[test]
fn unparseable_or_unknown_defaults_to_regression() {
    // Fail-safe: a broken/garbage classifier answer must never terminate a
    // real regression — it falls back to the retry path.
    assert_eq!(
        parse_triage_text("I could not decide."),
        TriageVerdict::Regression
    );
    assert_eq!(
        parse_triage_text(r#"{"category":"banana"}"#),
        TriageVerdict::Regression
    );
}

// ── whether a call is made at all ───────────────────────────────────────

/// The whole truth table. The property that matters is directional: no
/// combination of inputs may *cause* a triage call that the two guards did not
/// already both allow, because a call can end in `Environment`, which
/// terminates a run.
#[test]
fn only_a_reproduced_and_unsettled_failure_earns_a_call() {
    use TriageDecision::*;
    let cases = [
        // (prior fingerprint, current, triage_allowed, expected)
        (None, "fp", true, NotReproduced),
        (None, "fp", false, NotReproduced),
        (Some("other"), "fp", true, NotReproduced),
        (Some("other"), "fp", false, NotReproduced),
        (Some("fp"), "fp", false, SettledByBaseline),
        (Some("fp"), "fp", true, Consult),
    ];
    for (prior, current, allowed, expected) in cases {
        assert_eq!(
            triage_decision(prior, current, allowed),
            expected,
            "prior={prior:?} current={current} allowed={allowed}"
        );
    }
}

/// `Consult` is the only answer that spends tokens, and exactly one of the six
/// input combinations reaches it.
#[test]
fn withholding_is_the_only_thing_a_guard_can_do() {
    let inputs = [None, Some("fp"), Some("other")];
    let consults = inputs
        .into_iter()
        .flat_map(|p| [true, false].map(move |a| (p, a)))
        .filter(|(p, a)| triage_decision(*p, "fp", *a) == TriageDecision::Consult)
        .count();
    assert_eq!(consults, 1);
}

/// The fail-safe direction, stated as a property: every shape of unusable
/// classifier answer resolves to `Regression`, the retry path. `Environment`
/// terminates the run, so it may only come from an answer that says so.
#[test]
fn no_malformed_answer_can_terminate_a_run() {
    let corpus = [
        "",
        "   \n\t ",
        "I could not decide.",
        "{\"category\": \"envir",
        "{\"verdict\": \"environment\"}",
        r#"{"category":"banana"}"#,
        r#"{"category":123}"#,
        r#"["environment"]"#,
        "<think>environment</think>",
        "the category is environment",
    ];
    for raw in corpus {
        assert_eq!(
            parse_triage_text(raw),
            TriageVerdict::Regression,
            "an unusable answer must never terminate a run: {raw:?}"
        );
    }
}
