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
use crate::domain::harness_baseline::{
    BaselineEnvironmentFault, BaselineProducer, HarnessBaseline, HarnessBaselineRun,
};

const BASE: &str = "abc1234def5678";

fn measured(name: &str, exit_ok: bool, fingerprint: &str) -> HarnessBaselineRun {
    HarnessBaselineRun {
        name: name.to_string(),
        command: format!("npm run {name}"),
        exit_ok,
        fingerprint: fingerprint.to_string(),
        output_ref: Some(format!("/artifacts/{name}.log")),
        environment: None,
        failing_tests: None,
        measured_at: 1_000,
        producer: BaselineProducer::Node,
    }
}

/// A red measurement the classifier called *environmental* at measurement time:
/// the gate could not run on this machine, so its red says nothing about the
/// code. Indistinguishable from [`measured`]`(name, false, fp)` on every field
/// rungs 1 and 2 read — which is the whole reason rung 3 exists.
fn unrunnable(name: &str, fingerprint: &str) -> HarnessBaselineRun {
    HarnessBaselineRun {
        environment: Some(BaselineEnvironmentFault {
            reason: "pkg-config cannot find gdk-3.0".to_string(),
            remediation: "install libgtk-3-dev".to_string(),
        }),
        ..measured(name, false, fingerprint)
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
        // The overwhelmingly common shape: rung 3 costs an agent call, so the
        // caller pays for it only where it can act. Every test that is not about
        // rung 3 must behave as it did before the field existed.
        failing_tests: None,
    }
}

/// Rung 3's unscoped answer — a differently-red gate nobody could name the
/// delta of. Spelled once so the assertions below read as "new failures, no
/// scope" rather than as an empty-vector literal whose meaning has to be
/// re-derived.
fn unscoped() -> GateDetermination {
    GateDetermination::NewFailures {
        new_failures: Vec::new(),
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
    assert_eq!(
        d.outcome(),
        GateOutcome::Excluded,
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
    assert_eq!(
        d.outcome(),
        GateOutcome::Attributable,
        "a regression must fail the step"
    );
}

// ── The row that looks exactly like leg 1 and is its opposite ────────────────

/// A gate red at the base **because the machine cannot run it**, red now with
/// the identical output. Every input rung 2 can see is the same as leg 1's — the
/// same command, the same fingerprint, the same exit status both sides — so
/// without rung 3 this is subtracted and the step passes on a gate that verified
/// nothing.
///
/// The motivating incident was a missing `gdk-3.0`. It exits **1**, not 127, so
/// the exit-127 fast path never sees it; before decision 44 it reached C6's
/// classifier and terminated with remediation, and HB2c regressed that to a
/// silent pass. What tells it from leg 1 is not a comparison — it is the
/// classification the measurement recorded.
#[test]
fn a_gate_red_at_the_base_because_it_could_not_run_is_terminal_not_subtracted() {
    let base = record(BASE, vec![unrunnable("unit", "fp-unit")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(
        d,
        GateDetermination::Environment {
            reason: "pkg-config cannot find gdk-3.0".to_string(),
            remediation: "install libgtk-3-dev".to_string(),
        },
        "a gate that never ran proved nothing, so passing on it is evidence-free"
    );
    assert_eq!(
        d.outcome(),
        GateOutcome::Unrunnable {
            reason: "pkg-config cannot find gdk-3.0",
            remediation: "install libgtk-3-dev",
        },
        "and the remediation has to survive the comparison, or the terminal \
         failure cannot say what to install"
    );
}

/// The fail-safe direction, and the reason the field is `Option`-shaped rather
/// than an enum with an `Unknown` arm that a caller could mis-handle. A record
/// written before this field existed decodes with it absent, and *absent must
/// mean subtractable* — the pre-HB2c-fix behaviour — because only a positive
/// classification may terminate a run. A classifier that spawns, times out, or
/// answers unparseably lands here too.
#[test]
fn a_red_gate_that_was_never_classified_stays_pre_existing() {
    let base = record(BASE, vec![measured("unit", false, "fp-unit")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(d, GateDetermination::PreExisting);
    assert_eq!(d.outcome(), GateOutcome::Excluded);
}

/// The same claim reached through the decoder rather than through a constructor,
/// which is the only way to prove the *stored* shape of an older record. A
/// baseline written by a build that predates the field must decode cleanly and
/// behave as excluded — a decode failure would degrade to `NoBaseline`, and a
/// decode that defaulted the other way would terminate every run against an
/// already-red repository.
#[test]
fn a_record_written_before_the_field_existed_decodes_as_pre_existing() {
    let stored = r#"{"base_sha":"abc1234def5678","harnesses":[{"name":"unit",
        "command":"npm run unit","exit_ok":false,"fingerprint":"fp-unit",
        "measured_at":1000,"producer":"node"}]}"#;
    let base = HarnessBaseline::from_column(Some(stored)).expect("an older record must decode");

    assert!(
        base.harness("unit")
            .expect("the gate is there")
            .environment
            .is_none(),
        "absent is not environmental"
    );
    assert_eq!(
        determine(Some(&base), &observed("unit", "npm run unit", "fp-unit")),
        GateDetermination::PreExisting,
        "an unclassified red gate keeps the behaviour it had before the field existed"
    );
}

/// The classification is read *after* rung 2 has matched, not instead of it. A gate that was
/// unrunnable at the base and says something different now is not the failure
/// that was classified, so the classification is not evidence about it — it
/// stays a verdict, which is the safe direction (it withholds an escalation
/// rather than manufacturing one).
#[test]
fn an_environmental_baseline_does_not_terminate_a_differently_red_gate() {
    let base = record(BASE, vec![unrunnable("unit", "fp-old")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-new"));

    assert_eq!(d, unscoped());
    assert_eq!(d.outcome(), GateOutcome::Attributable);
}

/// And rung 1 still wins outright. A record cannot be both green and
/// environmental — a green gate is never classified — but if a malformed one
/// ever were, the exit status must still settle it, exactly as it does against a
/// fingerprint.
#[test]
fn a_green_baseline_is_a_regression_whatever_else_the_record_carries() {
    let mut green = measured("unit", true, "");
    green.environment = Some(BaselineEnvironmentFault {
        reason: "should never be read".to_string(),
        remediation: String::new(),
    });
    let base = record(BASE, vec![green]);

    assert_eq!(
        determine(Some(&base), &observed("unit", "npm run unit", "fp-new")),
        GateDetermination::Regression
    );
}

// ── The rest of the table ────────────────────────────────────────────────────

/// Red before, red now, but *differently*: new failures on top of a
/// pre-existing one. Still the feature's to answer for — something it did
/// changed what the gate says.
#[test]
fn a_differently_red_gate_is_new_failures_atop_the_pre_existing_one() {
    let base = record(BASE, vec![measured("unit", false, "fp-old")]);
    let d = determine(Some(&base), &observed("unit", "npm run unit", "fp-new"));

    assert_eq!(d, unscoped());
    assert_eq!(d.outcome(), GateOutcome::Attributable);
}

// ── Rung 3: which of those failures are actually new ─────────────────────────

/// A red measurement that also named the tests it reported failing.
fn named(name: &str, fingerprint: &str, tests: &[&str]) -> HarnessBaselineRun {
    HarnessBaselineRun {
        failing_tests: Some(tests.iter().map(|t| t.to_string()).collect()),
        ..measured(name, false, fingerprint)
    }
}

fn seen<'a>(
    name: &'a str,
    command: &'a str,
    fingerprint: &'a str,
    tests: &'a [String],
) -> ObservedFailure<'a> {
    ObservedFailure {
        failing_tests: Some(tests),
        ..observed(name, command, fingerprint)
    }
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// **The leg rung 3 exists for.** Rungs 1–2 can say only "red at the base, red
/// now, differently" — so the whole gate is the verdict and the rework cycle
/// re-derives all of it. With both sides' identifiers in hand, the delta is one
/// test, and the retry is scoped to it.
///
/// A refactor pipeline is the case that makes this load-bearing: "these 3 of 500
/// tests regressed" and "the suite is red" are the same determination and
/// completely different instructions.
#[test]
fn a_differently_red_gate_reports_only_the_failures_that_are_new() {
    let base = record(
        BASE,
        vec![named("unit", "fp-old", &["auth::expired", "db::pool"])],
    );
    let now = ids(&["auth::expired", "db::pool", "cart::totals"]);
    let d = determine(Some(&base), &seen("unit", "npm run unit", "fp-new", &now));

    assert_eq!(
        d,
        GateDetermination::NewFailures {
            new_failures: ids(&["cart::totals"]),
        },
        "only the failure absent from the base is this feature's to answer for"
    );
    assert_eq!(
        d.outcome(),
        GateOutcome::Attributable,
        "and it is still a verdict — rung 3 scopes one, it never converts one"
    );
}

/// The scope is not allowed to leak into any other row. Whatever the extractor
/// says, a gate that was **green** at the base is a regression and a gate that is
/// **identically** red is pre-existing — those were settled by rungs 1 and 2, and
/// a reading cannot reopen them.
#[test]
fn a_reading_cannot_change_a_determination_the_cheaper_rungs_settled() {
    let now = ids(&["cart::totals"]);

    let green = record(BASE, vec![measured("unit", true, "")]);
    assert_eq!(
        determine(Some(&green), &seen("unit", "npm run unit", "fp-new", &now)),
        GateDetermination::Regression,
        "rung 1 settled this; naming tests cannot make a regression pre-existing"
    );

    let same = record(BASE, vec![named("unit", "fp-unit", &["auth::expired"])]);
    assert_eq!(
        determine(Some(&same), &seen("unit", "npm run unit", "fp-unit", &now)),
        GateDetermination::PreExisting,
        "rung 2 settled this; a differing reading cannot manufacture a verdict"
    );

    let broken = record(BASE, vec![unrunnable("unit", "fp-unit")]);
    assert_eq!(
        determine(
            Some(&broken),
            &seen("unit", "npm run unit", "fp-unit", &now)
        ),
        GateDetermination::Environment {
            reason: "pkg-config cannot find gdk-3.0".to_string(),
            remediation: "install libgtk-3-dev".to_string(),
        },
        "and a gate that could not run is still terminal, whatever its output named"
    );
}

/// A malfunctioning extractor — one that answers nothing, on either side —
/// degrades to rung 2 with the determination untouched. That is the direction
/// the whole subsystem fails in: no reading costs a caller the pre-rung-3
/// behaviour, which is a correct if coarser answer.
#[test]
fn an_extraction_that_reads_nothing_degrades_to_rung_2() {
    let none: Vec<String> = Vec::new();

    // Nothing read now.
    let base = record(BASE, vec![named("unit", "fp-old", &["auth::expired"])]);
    assert_eq!(
        determine(Some(&base), &seen("unit", "npm run unit", "fp-new", &none)),
        unscoped(),
    );

    // Nothing read at the base — every name would otherwise read as new, which
    // is not a narrower statement but a fabricated one: the gate was already red,
    // so some of those failures certainly predate the feature.
    let unnamed = record(BASE, vec![measured("unit", false, "fp-old")]);
    let now = ids(&["auth::expired", "cart::totals"]);
    assert_eq!(
        determine(
            Some(&unnamed),
            &seen("unit", "npm run unit", "fp-new", &now)
        ),
        unscoped(),
    );
}

/// The delta is over the *live* order, deduplicated, and blind to surrounding
/// whitespace — a runner that prints a failing test in both a summary and a
/// detail block named one failure, not two.
#[test]
fn the_delta_is_the_live_order_deduplicated() {
    assert_eq!(
        new_failing_tests(
            &ids(&["a", " b "]),
            &ids(&["c", "b", "a", "c", "", "  ", "d"]),
        ),
        ids(&["c", "d"]),
    );
}

/// The predicate that decides whether an agent call is worth making. It is the
/// cost bound in one function: only a gate every cheaper rung has conceded, and
/// whose record holds something to diff against, is worth paying for.
#[test]
fn extraction_is_only_paid_for_where_it_could_narrow_something() {
    let cmp = |b: HarnessBaselineRun, fp: &str| {
        compare_gate(
            Some(&record(BASE, vec![b])),
            BASE,
            &observed("unit", "npm run unit", fp),
        )
        .extraction_would_scope()
    };

    assert!(
        cmp(named("unit", "fp-old", &["auth::expired"]), "fp-new"),
        "differently red, with names on record: the one case rung 3 can settle"
    );
    assert!(
        !cmp(measured("unit", false, "fp-old"), "fp-new"),
        "no names on record — an extraction could only fabricate scope"
    );
    assert!(
        !cmp(named("unit", "fp-unit", &["auth::expired"]), "fp-unit"),
        "rung 2 answered; a subtracted gate is not retried at all"
    );
    assert!(
        !cmp(measured("unit", true, ""), "fp-new"),
        "rung 1 answered; every failure on a green base is new by construction"
    );
    assert!(
        !compare_gate(None, BASE, &observed("unit", "npm run unit", "fp-new"))
            .extraction_would_scope(),
        "with no baseline there is nothing to subtract from"
    );
}

/// No record at all is today's behaviour, unchanged. This is the row that keeps
/// every project without a baseline exactly where it was.
#[test]
fn no_record_at_all_reproduces_todays_behaviour() {
    let d = determine(None, &observed("unit", "npm run unit", "fp-unit"));

    assert_eq!(d, GateDetermination::NoBaseline);
    assert_eq!(d.outcome(), GateOutcome::Attributable);
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

    assert_eq!(d, unscoped());
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
        !unscoped().allows_triage(),
        "the gate reached an exit status at the base and its output changed \
         under this feature: the measurement already answered"
    );
    assert!(
        !GateDetermination::PreExisting.allows_triage(),
        "a subtracted gate is never classified — there is no failure left"
    );
    assert!(
        !GateDetermination::Environment {
            reason: "no gdk".to_string(),
            remediation: "install libgtk-3-dev".to_string(),
        }
        .allows_triage(),
        "this one has already been classified, at measurement time — asking \
         again pays a second agent call for an answer already on the record"
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
