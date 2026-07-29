//! The subtraction, reached directly (HB2c, decision 44).
//!
//! Every test here calls the policy with values: no port doubles, no
//! `ExecutionDriver`, no I/O. That is the whole reason the comparison lives in
//! `domain/` — the two legs the task exists for (a gate red before and after
//! does **not** fail the step; a gate green before and red after **does**) are
//! decidable without provisioning a worktree or running a shell, and the
//! conformance gate in `tests/conformance/harness_subtraction.rs` then proves
//! the wiring reaches them.

use super::*;
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaseline, HarnessBaselineRun};

const BASE: &str = "abc1234def5678";

fn measured(name: &str, exit_ok: bool, fingerprint: &str) -> HarnessBaselineRun {
    HarnessBaselineRun {
        name: name.to_string(),
        command: format!("npm run {name}"),
        exit_ok,
        fingerprint: fingerprint.to_string(),
        output_ref: Some(format!("/artifacts/{name}.log")),
        environment: None,
        measured_at: 1_000,
        producer: BaselineProducer::Node,
    }
}

fn record(base_sha: &str, runs: Vec<HarnessBaselineRun>) -> HarnessBaseline {
    HarnessBaseline {
        base_sha: base_sha.to_string(),
        harnesses: runs,
    }
}

/// A live failure of `name`, running the same command the fixtures record.
fn observed<'a>(name: &'a str, command: &'a str, fingerprint: &'a str) -> ObservedFailure<'a> {
    ObservedFailure {
        name,
        command,
        fingerprint,
    }
}

fn determine(baseline: Option<&HarnessBaseline>, o: &ObservedFailure<'_>) -> GateDetermination {
    compare_gate(baseline, BASE, o).determination
}

// ── The two legs the task exists for ─────────────────────────────────────────

/// Leg 1. Red at the base with this exact failure, red now with it again: the
/// repository was already broken and this feature did not break it. Excluding
/// it is the entire point of decision 44 — without it, a run against an
/// already-red repo re-implements a correct feature at ~$14 and 11M tokens per
/// rework cycle.
#[test]
fn a_gate_red_before_and_identically_red_now_is_not_this_features_fault() {
    let base = record(BASE, vec![measured("unit", false, "fp-unit")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(d, GateDetermination::PreExisting);
    assert!(
        !d.is_attributable(),
        "a pre-existing failure must not fail the step"
    );
}

/// Leg 2. Green at the base, red now: the feature broke it, and the rework loop
/// is exactly where that belongs. No agent is consulted to reach this — the
/// measurement already decided it.
#[test]
fn a_gate_green_before_and_red_now_is_a_regression() {
    let base = record(BASE, vec![measured("unit", true, "")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-new"));

    assert_eq!(d, GateDetermination::Regression);
    assert!(d.is_attributable(), "a regression must fail the step");
}

// ── The rest of the table ────────────────────────────────────────────────────

/// Red before, red now, but *differently*: new failures on top of a
/// pre-existing one. Still the feature's to answer for — something it did
/// changed what the gate says.
#[test]
fn a_differently_red_gate_is_new_failures_atop_the_pre_existing_one() {
    let base = record(BASE, vec![measured("unit", false, "fp-old")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-new"));

    assert_eq!(d, GateDetermination::NewFailures);
    assert!(d.is_attributable());
}

/// No record at all is today's behaviour, unchanged. This is the row that keeps
/// every project without a baseline exactly where it was.
#[test]
fn no_record_at_all_reproduces_todays_behaviour() {
    let d = determine(None, &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(d, GateDetermination::NoBaseline);
    assert!(d.is_attributable());
}

// ── Absent is not green ──────────────────────────────────────────────────────

/// The inversion this whole module is built to prevent. A record that covers
/// the base but never measured *this* gate says nothing about it. Reading that
/// silence as "it was green" would turn every pre-existing failure of an
/// unmeasured gate into a fresh regression — the exact opposite of what the
/// baseline is for.
#[test]
fn a_gate_the_record_never_measured_is_unknown_not_green() {
    let base = record(BASE, vec![measured("lint", true, "")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(
        d,
        GateDetermination::NoBaseline,
        "an unmeasured gate must not borrow another gate's green"
    );
}

/// An empty record is the same silence, and must answer the same way.
#[test]
fn an_empty_record_answers_nothing_rather_than_fine() {
    let base = record(BASE, Vec::new());
    assert_eq!(
        determine(Some(&base), &observed("unit", "npm run unit", "fp")),
        GateDetermination::NoBaseline
    );
}

// ── A stale baseline is ignored, not trusted ─────────────────────────────────

/// `base_sha` exists so a measurement of *other code* is detectable. A record
/// taken against a different commit is not evidence about this run, however
/// complete it looks — and the failure mode of trusting it is silent: a gate
/// that was red two commits ago would excuse a regression introduced since.
#[test]
fn a_record_measured_against_another_commit_is_ignored() {
    let base = record("0000000000", vec![measured("unit", false, "fp-unit")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(
        d,
        GateDetermination::NoBaseline,
        "a stale baseline must be ignored rather than silently trusted"
    );
    assert!(
        compare_gate(
            Some(&base),
            BASE,
            &observed("unit", "npm run unit", "fp-unit")
        )
        .baseline
        .is_none(),
        "and it must not be offered as evidence either"
    );
}

/// A run whose merge-base would not resolve — or that was cancelled before it
/// could be asked — has no commit to check coverage against. It can cover
/// nothing, so it gets no subtraction.
#[test]
fn an_unresolvable_base_subtracts_nothing() {
    let base = record(BASE, vec![measured("unit", false, "fp-unit")]);
    let cmp = compare_gate(
        Some(&base),
        "   ",
        &observed("unit", "npm run unit", "fp-unit"),
    );

    assert_eq!(cmp.determination, GateDetermination::NoBaseline);
}

// ── A different command is a different question ──────────────────────────────

/// The record keeps the command string for exactly one purpose: `npm test` at
/// the base tells us nothing about `npm run test:ci` at the tip, however alike
/// the gate names are. Without this check, editing a project's command mid-run
/// would silently excuse whatever the new one does.
#[test]
fn a_baseline_of_a_different_command_is_not_a_comparison() {
    let base = record(BASE, vec![measured("unit", false, "fp-unit")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit:ci", "fp-unit"));

    assert_eq!(d, GateDetermination::NoBaseline);
}

/// Whitespace around a command is not a change of command — the two sides are
/// read from different places (a JSON map and a resolved harness) and one of
/// them trimming is not evidence about the base commit.
#[test]
fn whitespace_around_the_command_does_not_defeat_the_comparison() {
    let base = record(BASE, vec![measured("unit", false, "fp-unit")]);
    let d = determine(
        Some(&base),
        &observed("unit", "  npm run unit  ", "fp-unit"),
    );

    assert_eq!(d, GateDetermination::PreExisting);
}

// ── The ladder ───────────────────────────────────────────────────────────────

/// Rung 1 settles it outright: a gate that *passed* at the base is a regression
/// whatever its fingerprint now says, so no fingerprint comparison is even
/// attempted. A green baseline carries an empty fingerprint by construction, so
/// falling through to rung 2 with an equally empty live one would read as
/// "pre-existing" — the worst possible inversion.
#[test]
fn a_green_baseline_is_never_matched_by_fingerprint() {
    let base = record(BASE, vec![measured("unit", true, "")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", ""));

    assert_eq!(
        d,
        GateDetermination::Regression,
        "an empty-vs-empty fingerprint must not subtract a regression"
    );
}

/// The same guard from the other side: a *red* record with no fingerprint is a
/// shape we do not understand, and two blanks are not a match.
#[test]
fn a_red_baseline_without_a_fingerprint_matches_nothing() {
    let base = record(BASE, vec![measured("unit", false, "")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", ""));

    assert_eq!(d, GateDetermination::NewFailures);
}

// ── What C6's classifier is still asked about ────────────────────────────────

/// The narrowing. Triage exists to tell an unprovisioned machine from a broken
/// change; a covering baseline answers that as a measurement for two of the
/// four rows, and the classifier survives for the one that genuinely needs
/// judgement — green at the base, red now, for a reason that appeared during
/// the run.
#[test]
fn only_the_residue_still_needs_the_triage_agent() {
    assert!(
        GateDetermination::Regression.allows_triage(),
        "green-then-red is the residue C6 keeps: the fault may have appeared \
         during the run"
    );
    assert!(
        GateDetermination::NoBaseline.allows_triage(),
        "with no measurement there is nothing to narrow with — today's behaviour"
    );
    assert!(
        !GateDetermination::NewFailures.allows_triage(),
        "the gate reached an exit status at the base and its output changed \
         under this feature: the measurement already answered"
    );
    assert!(
        !GateDetermination::PreExisting.allows_triage(),
        "a subtracted gate is never classified — there is no failure left"
    );
}

// ── Several gates at once ────────────────────────────────────────────────────

/// A step with three red gates gets three independent answers, in the order the
/// gates ran (the declared order, cheap gates first — which is what the report
/// renders and the verdict names).
#[test]
fn every_gate_is_judged_on_its_own_evidence_in_order() {
    let base = record(
        BASE,
        vec![
            measured("lint", false, "fp-lint"),
            measured("unit", true, ""),
        ],
    );
    let observed = [
        observed("lint", "npm run lint", "fp-lint"),
        observed("unit", "npm run unit", "fp-unit"),
        observed("e2e", "npm run e2e", "fp-e2e"),
    ];

    let out = compare_gates(Some(&base), BASE, &observed);

    assert_eq!(
        out.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        vec!["lint", "unit", "e2e"],
        "order is the declared gate order and must survive"
    );
    assert_eq!(out[0].determination, GateDetermination::PreExisting);
    assert_eq!(out[1].determination, GateDetermination::Regression);
    assert_eq!(out[2].determination, GateDetermination::NoBaseline);
}

/// The evidence travels with the answer, so the caller can name *which* commit
/// and *which* producer excused a gate without re-opening the record. An
/// exclusion nobody can audit is one nobody will trust.
#[test]
fn an_excluded_gate_carries_the_evidence_that_excused_it() {
    let mut fallback = measured("unit", false, "fp-unit");
    fallback.producer = BaselineProducer::Fallback;
    let base = record(BASE, vec![fallback]);

    let cmp = compare_gate(
        Some(&base),
        BASE,
        &observed("unit", "npm run unit", "fp-unit"),
    );

    let evidence = cmp
        .baseline
        .expect("the excusing measurement travels along");
    assert_eq!(evidence.producer, BaselineProducer::Fallback);
    assert!(!evidence.exit_ok);
}
