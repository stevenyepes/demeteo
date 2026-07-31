// Tests for `src/domain/harness_attribution.rs` (mirrored-tests convention).
// `super` resolves to that module.
//
// Every assertion here was unreachable while the fold lived inside an `async fn`
// on `ExecutionDriver`: reaching it meant standing up eighteen ports it does not
// read. The four rules below are positional and each is one character from
// being silently wrong, so each is asserted on its own rather than through a
// single happy-path fixture that would pass with three of them broken.

use super::{split_by_determination, SubtractedFailures};
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaselineRun};
use crate::domain::harness_delta::{GateComparison, GateDetermination};
use crate::domain::harness_outcome::HarnessRun;

const BASE: &str = "abc1234def5678901234";

fn red(name: &str) -> HarnessRun {
    HarnessRun {
        name: name.to_string(),
        cmd: format!("npm run {name}"),
        output: format!("{name} is red"),
    }
}

fn measured(name: &str, producer: BaselineProducer) -> HarnessBaselineRun {
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

fn cmp(name: &str, determination: GateDetermination) -> GateComparison {
    GateComparison {
        name: name.to_string(),
        determination,
        baseline: Some(measured(name, BaselineProducer::Node)),
    }
}

fn unrunnable(reason: &str) -> GateDetermination {
    GateDetermination::Environment {
        reason: reason.to_string(),
        remediation: "install libgtk-3-dev".to_string(),
    }
}

fn split(gates: &[(&str, GateDetermination)]) -> SubtractedFailures {
    let runs: Vec<HarnessRun> = gates.iter().map(|(n, _)| red(n)).collect();
    let comparisons: Vec<GateComparison> = gates.iter().map(|(n, d)| cmp(n, d.clone())).collect();
    split_by_determination(&runs, &comparisons, BASE)
}

// ── rule 1: `triage_allowed` comes from the FIRST attributable gate ──────────

/// The classifier is asked about one gate — the first attributable one — so the
/// flag has to come from that gate's determination. Taking it from a later gate
/// would ask the agent a question the baseline already answered, or withhold one
/// it did not.
#[test]
fn triage_allowed_is_the_first_attributable_gates_answer_not_the_last() {
    // `Regression` allows triage; `NewFailures` does not. First wins both ways.
    let first_allows = split(&[
        ("lint", GateDetermination::Regression),
        (
            "unit",
            GateDetermination::NewFailures {
                new_failures: Vec::new(),
            },
        ),
    ]);
    assert!(first_allows.triage_allowed);

    let first_refuses = split(&[
        (
            "lint",
            GateDetermination::NewFailures {
                new_failures: Vec::new(),
            },
        ),
        ("unit", GateDetermination::Regression),
    ]);
    assert!(!first_refuses.triage_allowed);
}

/// An *excluded* gate in front of the attributable one must not set the flag:
/// it is subtracted before there is anything to classify, so its determination
/// answers a question nobody asks.
#[test]
fn an_excluded_gate_ahead_of_the_first_attributable_one_does_not_set_the_flag() {
    let out = split(&[
        ("lint", GateDetermination::PreExisting),
        ("unit", GateDetermination::Regression),
    ]);
    assert_eq!(out.attributable.len(), 1);
    assert!(
        out.triage_allowed,
        "the flag must be `unit`'s, and `unit` is a Regression"
    );
}

/// No attributable gate means nothing will be classified, so the flag is moot.
/// `true` keeps it from reading as a suppression that was never decided.
#[test]
fn an_all_excluded_set_defaults_to_allowing_triage() {
    let out = split(&[
        ("lint", GateDetermination::PreExisting),
        ("unit", GateDetermination::PreExisting),
    ]);
    assert!(out.attributable.is_empty());
    assert!(out.triage_allowed);
}

// ── rule 2: the unrunnable gate is the FIRST one only ───────────────────────

#[test]
fn only_the_first_unrunnable_gate_is_kept() {
    let out = split(&[
        ("lint", unrunnable("gdk-3.0 is missing")),
        ("unit", unrunnable("libssl is missing")),
    ]);
    let gate = out.unrunnable.expect("an unrunnable gate must survive");
    assert_eq!(
        gate.reason, "gdk-3.0 is missing",
        "the message carries one reproduce line, so it names one command"
    );
    assert_eq!(gate.run.name, "lint");
}

/// The two answers are independent: an `Unrunnable` in position 2 is still the
/// terminal decision, while position 1 has already set `triage_allowed`. A fold
/// that short-circuited on the first unrunnable gate would lose the flag; one
/// that let a later gate overwrite `triage_allowed` would lose the rule above.
#[test]
fn a_later_unrunnable_gate_still_terminates_while_the_first_gate_sets_the_flag() {
    let out = split(&[
        ("lint", GateDetermination::Regression),
        ("unit", unrunnable("gdk-3.0 is missing")),
    ]);

    assert!(out.unrunnable.is_some(), "position 2 still terminates");
    assert_eq!(out.attributable.len(), 1);
    assert!(
        out.triage_allowed,
        "position 1 is a Regression, and it is what set the flag"
    );
}

// ── rule 3: `new_failing_tests` is deduped in first-seen order ───────────────

#[test]
fn new_failing_tests_are_unioned_deduped_and_kept_in_first_seen_order() {
    let out = split(&[
        (
            "lint",
            GateDetermination::NewFailures {
                new_failures: vec!["b".to_string(), "a".to_string()],
            },
        ),
        (
            "unit",
            GateDetermination::NewFailures {
                new_failures: vec!["a".to_string(), "c".to_string()],
            },
        ),
    ]);
    assert_eq!(out.new_failing_tests, vec!["b", "a", "c"]);
}

/// Only the attributable gates contribute scope. A determination that is not a
/// verdict has no failures to name — and an empty list means *unscoped*, never
/// "nothing failed", so it must not be filled from a gate that was subtracted.
#[test]
fn an_excluded_gate_contributes_no_scope() {
    let out = split(&[
        ("lint", GateDetermination::PreExisting),
        ("unit", GateDetermination::Regression),
    ]);
    assert!(out.new_failing_tests.is_empty());
}

// ── the exclusion is worded where the failure would have been ───────────────

/// The subtraction has to be auditable, and the two producers have very
/// different stories: the node measured at the head of the run, the fallback
/// measured on this very failure path.
#[test]
fn an_excluded_gate_is_worded_with_the_producer_that_measured_it() {
    let runs = vec![red("lint")];
    let node = split_by_determination(
        &runs,
        &[GateComparison {
            name: "lint".to_string(),
            determination: GateDetermination::PreExisting,
            baseline: Some(measured("lint", BaselineProducer::Node)),
        }],
        BASE,
    );
    assert!(node.excluded[0].reason.contains("head of this run"));
    assert!(node.excluded[0].reason.contains("abc1234def56"));

    let fallback = split_by_determination(
        &runs,
        &[GateComparison {
            name: "lint".to_string(),
            determination: GateDetermination::PreExisting,
            baseline: Some(measured("lint", BaselineProducer::Fallback)),
        }],
        BASE,
    );
    assert!(
        fallback.excluded[0].reason.contains("failure path"),
        "the two producers must be distinguishable; got: {}",
        fallback.excluded[0].reason
    );
}

// ── the zip ─────────────────────────────────────────────────────────────────

/// Every caller derives `comparisons` from `failed`, so the two are the same
/// length in production — but the fold must not panic when they are not, and an
/// empty comparison slice is the shape a cancelled extraction could produce.
#[test]
fn mismatched_slice_lengths_truncate_rather_than_panic() {
    let runs = vec![red("lint"), red("unit")];
    let out = split_by_determination(&runs, &[], BASE);
    assert!(out.attributable.is_empty());
    assert!(out.excluded.is_empty());
    assert!(out.unrunnable.is_none());
    assert!(out.triage_allowed);

    let one = split_by_determination(&runs, &[cmp("lint", GateDetermination::Regression)], BASE);
    assert_eq!(one.attributable.len(), 1);
}
