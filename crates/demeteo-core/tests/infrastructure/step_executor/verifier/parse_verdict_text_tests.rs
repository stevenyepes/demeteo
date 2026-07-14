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

// ── brace recovery: the object is there, one brace is not ────────────────

#[test]
fn recovers_a_verdict_object_that_lost_its_opening_brace() {
    // Real MiniMax-M3 output from a detached validate turn: every field the
    // contract asks for, closing `}` present, opening `{` never emitted. The
    // turn also quotes TypeScript containing `{ kind: "ssh" }`, which is what
    // the old first-`{`-to-last-`}` fallback latched onto — it reported a parse
    // error about *that* snippet and threw the whole step away.
    let text =
        "I reviewed the diff. `formatError({ kind: 'ssh' })` still returns '[object Object]'.\n\n\
                \"verdict\": \"fail\", \"reason\": \"no fallback for AppError-shaped objects\", \
                \"failing_tests\": [\"src/lib/errors.test.ts case 4\"], \
                \"implicated_files\": [\"src/lib/errors.ts\"] }";
    match parse_verdict_text(text, "verdict") {
        ParsedVerdict::Fail(vf) => {
            assert_eq!(vf.reason, "no fallback for AppError-shaped objects");
            assert_eq!(vf.implicated_files, vec!["src/lib/errors.ts"]);
        }
        other => panic!("expected the verdict to be recovered, got {:?}", other),
    }
}

#[test]
fn recovers_a_verdict_object_that_lost_its_closing_brace() {
    let text = "{\"verdict\": \"pass\"";
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Pass
    ));
}

#[test]
fn recovery_ignores_trailing_prose_after_the_object() {
    let text = "\"verdict\": \"pass\" }\n\nThat's my assessment — the change is correct.";
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Pass
    ));
}

#[test]
fn recovery_cannot_be_faked_by_prose_naming_the_key() {
    // The key has to open a parseable object. A turn that merely *mentions*
    // "verdict" — even next to a stray brace — stays Missing rather than
    // inventing a pass/fail the model never gave.
    let text = "I could not run the tests, so I am withholding my \"verdict\" on this one }";
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Missing(_)
    ));
}

#[test]
fn missing_verdict_error_quotes_the_turn_not_a_stitched_span() {
    // The old message pasted serde's complaint about a span stitched from the
    // first `{` in the turn to the last `}`, so on any turn quoting code it
    // described the wrong brace entirely. The message must name the key it
    // wanted and show how the turn actually ended.
    let text = "Here is the patch:\n\nconst x = { kind: \"ssh\" };\n\nLooks good to me!";
    match parse_verdict_text(text, "verdict") {
        ParsedVerdict::Missing(desc) => {
            assert!(desc.contains("verdict"), "should name the key: {desc}");
            assert!(
                desc.contains("Looks good to me!"),
                "should quote the turn's tail: {desc}"
            );
            assert!(
                !desc.contains("line 1 column"),
                "should not paste a serde error about a stitched span: {desc}"
            );
        }
        other => panic!("expected Missing, got {:?}", other),
    }
}

#[test]
fn think_tags_are_stripped_before_parsing() {
    let text = "<think>{\"verdict\": \"fail\"} draft</think>{\"verdict\": \"pass\"}";
    assert!(matches!(
        parse_verdict_text(text, "verdict"),
        ParsedVerdict::Pass
    ));
}
