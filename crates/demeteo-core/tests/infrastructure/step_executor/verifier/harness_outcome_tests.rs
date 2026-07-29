// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// These cover the three decisions the validate prompt makes about *evidence*:
// what a harness block claims (S12), whether a green run's output survives at
// all (S11), and which verdicts the agent is actually offered (S13). All three
// were previously spelled inside `async fn`s that also did I/O, so none of them
// could be asserted without standing up a driver and twenty ports it never read.

use super::{
    build_exclusion_note, build_exclusion_reason, build_failure_reason, merge_stderr_into_stdout,
    verdict_contract, ExcludedRun, HarnessOutcome, HarnessRun,
};
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaselineRun};

fn run(name: &str, cmd: &str, output: &str) -> HarnessRun {
    HarnessRun {
        name: name.to_string(),
        cmd: cmd.to_string(),
        output: output.to_string(),
    }
}

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
    let rendered = HarnessOutcome::from_runs(vec![run(
        "default",
        "cargo test",
        "test result: ok. 57 passed",
    )])
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
    let ran = HarnessOutcome::from_runs(vec![run("default", "true", "")]).render_section();
    let absent = HarnessOutcome::NotConfigured.render_section();

    let heading = |s: &str| s.lines().next().unwrap_or_default().to_string();
    assert_ne!(heading(&ran), heading(&absent));
}

// ── HB5: several gates, each attributable ────────────────────────────────────

#[test]
fn every_harness_gets_its_own_named_block() {
    // The whole point of the list. `&&`-chaining two commands hands the agent
    // one undifferentiated blob in which "which gate produced this line" is
    // unanswerable — and that attribution is what a per-gate verdict needs.
    let rendered = HarnessOutcome::from_runs(vec![
        run("lint", "npm run lint", "0 problems"),
        run("unit", "npm test", "42 passing"),
    ])
    .render_section();

    for (name, cmd, output) in [
        ("lint", "npm run lint", "0 problems"),
        ("unit", "npm test", "42 passing"),
    ] {
        assert!(
            rendered.contains(&format!("### Harness `{name}`")),
            "every gate needs its own labelled heading; {name} missing from:\n{rendered}"
        );
        assert!(rendered.contains(cmd), "missing {cmd} in:\n{rendered}");
        assert!(
            rendered.contains(output),
            "missing {output} in:\n{rendered}"
        );
    }
    // Declared order is the user's order (cheap gates first), so the blocks
    // must not be reordered on the way into the prompt.
    assert!(
        rendered.find("### Harness `lint`") < rendered.find("### Harness `unit`"),
        "blocks must follow declared order; got:\n{rendered}"
    );
}

#[test]
fn no_runs_at_all_is_not_configured_rather_than_an_empty_pass() {
    // The constructor is the only thing standing between "we ran nothing" and
    // a `Ran([])` that would render the authoritative heading over no evidence
    // at all — the S12 bug with an extra step.
    assert!(matches!(
        HarnessOutcome::from_runs(Vec::new()),
        HarnessOutcome::NotConfigured
    ));
    let rendered = HarnessOutcome::from_runs(Vec::new()).render_section();
    assert!(rendered.contains("NOTHING RAN"));
    assert!(!rendered.contains("already executed"));
}

// ── HB5: a failure says which gate went red ──────────────────────────────────

#[test]
fn a_failure_names_the_harness_that_failed() {
    let reason = build_failure_reason(&[run("lint", "npm run lint", "3 problems")]);

    assert!(
        reason.contains("'lint'"),
        "the retry feedback must name the gate, not just the command; got:\n{reason}"
    );
    assert!(reason.contains("npm run lint"));
    assert!(
        reason.contains("exited with failure"),
        "the wording every consumer of this string matches on must survive"
    );
}

#[test]
fn both_failing_harnesses_reach_the_retry_feedback() {
    // If only the first red gate reached the implementer it would fix that one
    // and rediscover the second on the next cycle — one wasted cycle turned
    // into two, which is exactly what running every declared harness exists to
    // prevent. Reporting only half of what ran would give the saving back.
    let reason = build_failure_reason(&[
        run("lint", "npm run lint", "3 problems"),
        run("unit", "npm test", "1 failing: adds two numbers"),
    ]);

    assert!(reason.contains("'lint'") && reason.contains("3 problems"));
    assert!(reason.contains("'unit'") && reason.contains("1 failing: adds two numbers"));
    assert!(
        reason.contains("2 of this step's harnesses failed"),
        "the count must lead, so the reader knows to look for more than one; got:\n{reason}"
    );
}

#[test]
fn a_single_failure_reads_exactly_as_it_did_before_the_list() {
    // Back-compat where it is observable: one red gate must not acquire a
    // "1 of this step's harnesses failed" preamble it never had.
    let reason = build_failure_reason(&[run("default", "cargo test", "boom")]);
    assert!(!reason.contains("harnesses failed"));
    assert_eq!(
        reason,
        "'default' — command 'cargo test' exited with failure:\nboom"
    );
}

#[test]
fn the_tail_budget_is_shared_not_multiplied() {
    // A step with five red gates must not grow the retry prompt fivefold. Each
    // gate still gets a floor worth of tail (enough for a stack), and the
    // failing *end* of each output is what survives — the assertion, not the
    // build banner.
    let long = "x".repeat(10_000);
    let five: Vec<_> = ["a", "b", "c", "d", "e"]
        .iter()
        .map(|n| run(n, "cmd", &format!("{long}TAIL-{n}")))
        .collect();
    let reason = build_failure_reason(&five);

    assert!(
        reason.len() < 5 * 2000,
        "budget must be shared; got {} chars",
        reason.len()
    );
    for n in ["a", "b", "c", "d", "e"] {
        assert!(
            reason.contains(&format!("TAIL-{n}")),
            "every gate keeps the tail of its own output; {n} lost"
        );
    }
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

// ── HB2c: a subtraction the user can audit ───────────────────────────────────

fn measured_at_base(name: &str, producer: BaselineProducer) -> HarnessBaselineRun {
    HarnessBaselineRun {
        name: name.to_string(),
        command: format!("npm run {name}"),
        exit_ok: false,
        fingerprint: "fp".to_string(),
        output_ref: None,
        measured_at: 1_700_000_000,
        producer,
    }
}

fn excluded(name: &str, cmd: &str, output: &str) -> ExcludedRun {
    ExcludedRun {
        run: run(name, cmd, output),
        reason: build_exclusion_reason(
            "abc1234def5678901234",
            Some(&measured_at_base(name, BaselineProducer::Node)),
        ),
    }
}

/// The requirement decision 44 attaches to the subtraction itself: it does not
/// fail the step, but it must be *named*. A red gate that quietly vanishes from
/// the prompt is a verdict the reader cannot audit — and the first time the
/// subtraction is wrong, nothing in the report will say it happened.
#[test]
fn an_excluded_gate_is_named_in_the_prompt_with_its_output() {
    let rendered = HarnessOutcome::from_runs_with_exclusions(
        vec![run("unit", "npm run unit", "42 passing")],
        vec![excluded("lint", "npm run lint", "3 problems")],
    )
    .render_section();

    assert!(
        rendered.contains("### Harness `unit`"),
        "the passing gate still renders; got:\n{rendered}"
    );
    assert!(
        rendered.contains("Excluded"),
        "the exclusion needs a heading of its own, not a footnote; got:\n{rendered}"
    );
    assert!(
        rendered.contains("lint") && rendered.contains("3 problems"),
        "the excluded gate is named and its output shown; got:\n{rendered}"
    );
    assert!(
        rendered.contains("Record each excluded gate in your report"),
        "naming it in the prompt is only half the audit trail — the report has \
         to carry it too; got:\n{rendered}"
    );
    assert!(
        rendered.contains("not") && rendered.contains("implementation defect"),
        "and the agent must be told not to treat it as one; got:\n{rendered}"
    );
}

/// A pass whose *every* gate was excluded still ran commands. Collapsing it
/// into the no-harness block would tell the agent nothing executed — S12's bug
/// arriving from the other direction, and with the same consequence: a verdict
/// reached on evidence the prompt disclaims.
#[test]
fn a_run_whose_every_gate_was_excluded_is_not_the_no_harness_block() {
    let outcome = HarnessOutcome::from_runs_with_exclusions(
        Vec::new(),
        vec![excluded("lint", "npm run lint", "3 problems")],
    );
    let rendered = outcome.render_section();

    assert!(matches!(outcome, HarnessOutcome::Ran { .. }));
    assert!(
        !rendered.contains("NOTHING RAN"),
        "something did run; got:\n{rendered}"
    );
    assert!(rendered.contains("3 problems"), "got:\n{rendered}");
}

/// Nothing subtracted must render exactly as it did before HB2c — the
/// overwhelmingly common case, and the one every existing prompt expectation
/// was written against.
#[test]
fn no_exclusions_renders_byte_for_byte_as_before() {
    let before =
        HarnessOutcome::from_runs(vec![run("unit", "npm run unit", "42 passing")]).render_section();
    let after = HarnessOutcome::from_runs_with_exclusions(
        vec![run("unit", "npm run unit", "42 passing")],
        Vec::new(),
    )
    .render_section();

    assert_eq!(before, after);
    assert!(!before.contains("Excluded"), "got:\n{before}");
}

/// The evidence has to be inspectable: which commit, and which producer
/// measured it. "Trust me" is not an audit trail, and the two producers have
/// very different stories a support question will ask about.
#[test]
fn the_exclusion_reason_names_the_commit_and_the_producer() {
    let node = build_exclusion_reason(
        "abc1234def5678901234",
        Some(&measured_at_base("lint", BaselineProducer::Node)),
    );
    assert!(node.contains("abc1234def56"), "got: {node}");
    assert!(
        !node.contains("abc1234def5678901234"),
        "the full sha is unreadable inline; got: {node}"
    );
    assert!(node.contains("head of this run"), "got: {node}");

    let fallback = build_exclusion_reason(
        "abc1234def5678901234",
        Some(&measured_at_base("lint", BaselineProducer::Fallback)),
    );
    assert!(
        fallback.contains("failure path"),
        "the two producers must be distinguishable; got: {fallback}"
    );
}

/// The rework loop reads the verdict reason and turns it into tickets. An
/// implementer told "`unit` failed", who can also see a red `lint` in the log,
/// has every reason to go and fix `lint` too — work nobody asked for, on a
/// defect this feature did not cause.
#[test]
fn the_verdict_reason_names_what_it_is_not_asking_for() {
    let note = build_exclusion_note(&[excluded("lint", "npm run lint", "3 problems")]);

    assert!(note.contains("'lint'"), "got: {note}");
    assert!(note.contains("NOT part of this verdict"), "got: {note}");
    assert!(note.contains("do not try to fix"), "got: {note}");
}

#[test]
fn nothing_excluded_adds_nothing_to_the_verdict_reason() {
    assert_eq!(build_exclusion_note(&[]), "");
    // And the reason a single failure produces is unchanged, which is what
    // every existing retry-feedback expectation was written against.
    let reason = build_failure_reason(&[run("unit", "npm run unit", "boom")]);
    assert_eq!(format!("{reason}{}", build_exclusion_note(&[])), reason);
}
