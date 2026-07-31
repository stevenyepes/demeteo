// Tests for `src/domain/harness_outcome.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// HB2c's half: a gate the baseline excused is subtracted from the verdict, and
// the subtraction has to be auditable everywhere the failure would have been —
// the prompt section, and the verdict reason the rework loop reads.

use super::{
    build_exclusion_note, build_exclusion_reason, build_failure_reason, ExcludedRun,
    HarnessOutcome, HarnessRun,
};
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaselineRun};

fn run(name: &str, cmd: &str, output: &str) -> HarnessRun {
    HarnessRun {
        name: name.to_string(),
        cmd: cmd.to_string(),
        output: output.to_string(),
    }
}

fn measured_at_base(name: &str, producer: BaselineProducer) -> HarnessBaselineRun {
    HarnessBaselineRun {
        name: name.to_string(),
        command: format!("npm run {name}"),
        exit_ok: false,
        fingerprint: "fp".to_string(),
        output_ref: None,
        environment: None,
        failing_tests: None,
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

/// An excluded gate renders its full block too, so the budget has to cover it —
/// and be shared with the passing gates rather than paid on top of them.
#[test]
fn an_excluded_gates_output_is_windowed_on_the_same_shared_budget() {
    let huge = format!("HEAD-lint\n{}\nTAIL-lint\n", "noise\n".repeat(40_000));
    let rendered = HarnessOutcome::from_runs_with_exclusions(
        vec![run("unit", "npm run unit", &huge.replace("lint", "unit"))],
        vec![excluded("lint", "npm run lint", &huge)],
    )
    .render_section();

    assert!(
        rendered.len() < crate::domain::prompt_budget::ARGV_STRING_LIMIT_BYTES,
        "two gates rendered {} bytes",
        rendered.len()
    );
    for marker in ["HEAD-unit", "TAIL-unit", "HEAD-lint", "TAIL-lint"] {
        assert!(rendered.contains(marker), "{marker} lost from:\n…");
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
