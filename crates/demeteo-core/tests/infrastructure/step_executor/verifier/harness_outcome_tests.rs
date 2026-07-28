// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// These cover the three decisions the validate prompt makes about *evidence*:
// what a harness block claims (S12), whether a green run's output survives at
// all (S11), and which verdicts the agent is actually offered (S13). All three
// were previously spelled inside `async fn`s that also did I/O, so none of them
// could be asserted without standing up a driver and twenty ports it never read.

use super::{merge_stderr_into_stdout, verdict_contract, HarnessOutcome};

// ── S12: an absent harness must not read as a passing one ────────────────────

#[test]
fn absent_harness_never_claims_anything_was_executed() {
    let rendered = HarnessOutcome::NotConfigured.render_section();

    // The exact phrase that made this a bug: the fallback string used to be
    // printed under the caller's `## Harness Results (already executed by the
    // orchestrator)` heading. An agent told nothing ran, that the nothing is
    // authoritative, and that it may not check, certifies a feature nobody
    // tested.
    assert!(
        !rendered.contains("already executed"),
        "the no-harness block must not claim execution; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("authoritative"),
        "nothing ran, so there is nothing to call authoritative; got:\n{rendered}"
    );
    // And it must not carry the ban that only makes sense for a real result —
    // otherwise the agent is forbidden from establishing anything at all.
    assert!(
        !rendered.contains("Do NOT re-run"),
        "the ban belongs to the ran-harness block only; got:\n{rendered}"
    );

    assert!(rendered.contains("NOTHING RAN"));
    assert!(
        rendered.contains("environment"),
        "an unevidenceable criterion is a config problem — the block must point \
         at the verdict that says so rather than leaving `fail` as the only exit"
    );
}

#[test]
fn ran_harness_carries_its_output_and_the_ban() {
    let rendered = HarnessOutcome::Ran {
        name: "default".into(),
        cmd: "cargo test".into(),
        output: "test result: ok. 57 passed".into(),
    }
    .render_section();

    assert!(rendered.contains("already executed by the orchestrator"));
    assert!(rendered.contains("cargo test"));
    assert!(rendered.contains("test result: ok. 57 passed"));
    assert!(rendered.contains("Do NOT re-run"));
}

#[test]
fn the_two_outcomes_share_no_misleading_wording() {
    // The headings must be distinguishable at a glance in a long prompt: this
    // is the one signal the agent has for "is there evidence here or not".
    let ran = HarnessOutcome::Ran {
        name: "default".into(),
        cmd: "true".into(),
        output: String::new(),
    }
    .render_section();
    let absent = HarnessOutcome::NotConfigured.render_section();

    let heading = |s: &str| s.lines().next().unwrap_or_default().to_string();
    assert_ne!(heading(&ran), heading(&absent));
}

// ── S11: a green run's stderr must survive ───────────────────────────────────

#[test]
fn merge_wraps_in_a_subshell_redirecting_stderr() {
    assert_eq!(
        merge_stderr_into_stdout("cargo test"),
        "(\ncargo test\n) 2>&1"
    );
}

#[test]
fn merge_survives_a_command_ending_in_a_comment() {
    // The newlines are not cosmetic. `(cargo test # note) 2>&1` comments out
    // the closing paren and the redirect, turning valid shell into a syntax
    // error — and the harness command is user-authored, so this is reachable.
    let wrapped = merge_stderr_into_stdout("cargo test # run the suite");
    assert!(
        wrapped.ends_with("\n) 2>&1"),
        "closing paren must sit on its own line; got: {wrapped}"
    );
}

#[test]
fn merge_preserves_multi_command_harnesses() {
    // The shape `detect_worktree_strategy` emits for a polyglot repo. The
    // subshell must not disturb the `exit $rc` accumulator that makes the
    // combined status meaningful.
    let cmd = "set +e; rc=0; npm test; rc=$((rc||$?)); cargo test; rc=$((rc||$?)); exit $rc";
    let wrapped = merge_stderr_into_stdout(cmd);
    assert!(wrapped.contains(cmd));
    assert!(wrapped.starts_with("(\n") && wrapped.ends_with("\n) 2>&1"));
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
