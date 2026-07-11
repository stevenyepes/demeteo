// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::{parse_verdict_text, ParsedVerdict};

#[test]
fn pass_verdict_amid_prose() {
    let text = "Report written to artifacts/validation-report.md.\n\n{ \"verdict\": \"pass\" }";
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Pass
    ));
}

#[test]
fn fail_verdict_carries_structured_fields() {
    let text = r#"Done. {"verdict": "fail", "reason": "auth test broken", "failing_tests": ["auth::login_works"], "implicated_files": ["src/auth.rs"]}"#;
    match parse_verdict_text(text, "verdict") {
        ParsedVerdict::Fail(vf) => {
            assert_eq!(vf.reason, "auth test broken");
            assert_eq!(vf.failing_tests, vec!["auth::login_works"]);
            assert_eq!(vf.implicated_files, vec!["src/auth.rs"]);
        }
        _ => panic!("expected fail verdict"),
    }
}

#[test]
fn fail_without_lists_defaults_to_empty() {
    let text = r#"{"verdict": "fail", "reason": "nope"}"#;
    match parse_verdict_text(text, "verdict") {
        ParsedVerdict::Fail(vf) => {
            assert!(vf.failing_tests.is_empty());
            assert!(vf.implicated_files.is_empty());
        }
        _ => panic!("expected fail verdict"),
    }
}

#[test]
fn nested_verdict_object_is_found() {
    let text = r#"{"result": {"verdict": "pass"}}"#;
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Pass
    ));
}

#[test]
fn missing_verdict_reports_missing() {
    assert!(matches!(
        parse_verdict_text("all good, ship it!", "verdict"),
        ParsedVerdict::Missing(_)
    ));
}

#[test]
fn invalid_verdict_value_reports_missing() {
    assert!(matches!(
        parse_verdict_text(r#"{"verdict": "maybe"}"#, "verdict"),
        ParsedVerdict::Missing(_)
    ));
}

#[test]
fn think_tags_are_stripped_before_parsing() {
    let text = "<think>{\"verdict\": \"fail\"} draft</think>{\"verdict\": \"pass\"}";
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Pass
    ));
}
