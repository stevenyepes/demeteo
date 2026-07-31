// Tests for `src/domain/verifier/verdict.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// Ask and parse are one contract, so they are asserted together: the menu of
// verdicts `verdict_contract` offers must be the set `parse_verdict_text`
// accepts, and S13 is what happened the one time they were not.

use super::{build_verifier_prompt, parse_verdict_text, verdict_contract, ParsedVerdict};

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

// --- the `environment` verdict -----------------------------------------------
//
// A third answer, not a flavour of `fail`. `fail` opens a rework loop, which
// is right when an agent can fix what is broken — and wrong when the
// unsatisfied criteria demand a command the *project* is not configured to
// run. Nothing an agent writes adds a `build_command` to project settings,
// so routing that to `fail` spends the whole retry budget re-implementing a
// feature that was already correct.

#[test]
fn environment_verdict_carries_its_remediation() {
    let text = r#"Report written. {"verdict": "environment", "reason": "Criterion 1 requires `npm run build`, which this project's harness does not run. Set build_command in project settings."}"#;
    match parse_verdict_text(text, "verdict") {
        ParsedVerdict::Environment(reason) => {
            assert!(reason.contains("build_command"), "{reason}");
        }
        other => panic!("expected environment verdict, got {other:?}"),
    }
}

#[test]
fn environment_verdict_without_a_reason_still_says_something_actionable() {
    let text = r#"{"verdict": "environment"}"#;
    match parse_verdict_text(text, "verdict") {
        ParsedVerdict::Environment(reason) => assert!(!reason.trim().is_empty()),
        other => panic!("expected environment verdict, got {other:?}"),
    }
}

#[test]
fn environment_is_recognised_case_insensitively_like_the_others() {
    assert!(matches!(
        parse_verdict_text(r#"{"verdict": "ENVIRONMENT"}"#, "verdict"),
        ParsedVerdict::Environment(_)
    ));
}

#[test]
fn an_unknown_verdict_word_is_still_missing_not_environment() {
    // The vocabulary stays closed: only the three words route anywhere.
    match parse_verdict_text(r#"{"verdict": "maybe"}"#, "verdict") {
        ParsedVerdict::Missing(desc) => assert!(desc.contains("maybe"), "{desc}"),
        other => panic!("expected missing, got {other:?}"),
    }
}

// ── S13: the agent must be offered the verdict that fits a config defect ─────

#[test]
fn verdict_contract_offers_all_three_verdicts() {
    let contract = verdict_contract("verdict");

    assert!(contract.contains("\"verdict\": \"pass\""));
    assert!(contract.contains("\"verdict\": \"fail\""));
    // The one that was missing. `parse_verdict_text` has always accepted it and
    // the shipped verifier instructions have always asked for it, but this menu
    // listed only pass and fail — so an agent that had correctly judged a
    // criterion unprovable still had to answer `fail`, and `fail` opens a
    // rework loop against a feature whose defect is a project setting.
    assert!(
        contract.contains("\"verdict\": \"environment\""),
        "environment must be in the menu, not only in the prose instructions; got:\n{contract}"
    );
}

#[test]
fn verdict_contract_explains_when_environment_beats_fail() {
    // Offering the option is not enough — the model needs the discriminator,
    // because `fail` is the more natural reading of "a criterion is not met".
    let contract = verdict_contract("verdict");
    assert!(contract.contains("NOT `fail`"));
    assert!(contract.contains("rework budget"));
}

#[test]
fn verdict_contract_honours_a_custom_verdict_key() {
    // `VerifierConfig::verdict_key` is configurable and `parse_verdict_text`
    // reads whatever it says; a hard-coded key here would silently produce a
    // contract the parser cannot satisfy.
    let contract = verdict_contract("ship_it");
    assert!(contract.contains("\"ship_it\": \"pass\""));
    assert!(contract.contains("\"ship_it\": \"environment\""));
    assert!(!contract.contains("\"verdict\":"));
}

// ── the dedicated verifier turn's prompt ─────────────────────────────────────

/// The two properties the turn used to prove only by inspection, because the
/// `format!` lived inside a 250-line `async fn`.
///
/// The key has to appear three times — once as the requirement and twice in the
/// worked examples — or a custom `verdict_key` produces a prompt whose examples
/// contradict its own instruction. And the harness section goes in *verbatim*:
/// it carries its own heading and its own claim about whether anything ran, so
/// anything this template did to it would be the S12 coupling arriving from the
/// prompt side.
#[test]
fn the_verifier_prompt_carries_the_key_three_times_and_the_harness_section_whole() {
    let section = "## Harness Results — NOTHING RAN\nnothing was executed.\n";
    let prompt = build_verifier_prompt(
        "Judge the acceptance criteria.",
        section,
        "- File/Artifact: report.md\n",
        "ship_it",
    );

    assert_eq!(
        prompt.matches("ship_it").count(),
        3,
        "the requirement and both worked examples must name the same key; got:\n{prompt}"
    );
    assert!(
        prompt.contains(section),
        "the harness section must survive byte-for-byte; got:\n{prompt}"
    );
    assert!(prompt.contains("Judge the acceptance criteria."));
    assert!(prompt.contains("- File/Artifact: report.md"));
    assert!(
        !prompt.contains("\"verdict\""),
        "no default key may leak past a custom one; got:\n{prompt}"
    );
}
