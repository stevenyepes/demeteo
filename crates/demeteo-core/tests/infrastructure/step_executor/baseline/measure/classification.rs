//! Why a red gate is red, and what it named — each asked once, at measurement
//! time, and only of a gate that is actually red.

use super::*;

// ── Classifying a red gate, once, at measurement time ────────────────────────

#[tokio::test]
async fn a_red_gate_the_machine_cannot_run_is_recorded_as_environmental() {
    // The regression this exists for. `gdk-3.0` missing exits **1**, not 127,
    // so the fast path cannot see it — and to HB2c's fingerprint comparison it
    // is indistinguishable from a pre-existing test failure. The record is the
    // only place that difference can live.
    let exec = scripted(&[(
        "cargo test",
        Err("error: failed to run custom build command"),
    )]);
    let triage = ScriptedTriage::new(&[(
        "unit",
        environmental("pkg-config cannot find gdk-3.0", "install libgtk-3-dev"),
    )]);
    let measured = measure_with(&exec, &triage, None, &[gate("unit", "cargo test", 600)]).await;

    let fault = measured[0]
        .run
        .environment
        .as_ref()
        .expect("an unrunnable gate must be recorded as one");
    assert_eq!(fault.reason, "pkg-config cannot find gdk-3.0");
    assert_eq!(
        fault.remediation, "install libgtk-3-dev",
        "the remediation is what makes the terminal failure actionable"
    );
    assert!(
        !measured[0].run.exit_ok,
        "and it is still a red measurement — the classification says why, not whether"
    );
}

// ── Reading a red gate's test identifiers, once, at measurement time ─────────

#[tokio::test]
async fn a_red_gate_records_the_test_identifiers_it_named() {
    // The record HB2c's rung 3 diffs against, and the granularity the refactor
    // pipeline's per-test comparison needs: "these 3 of 500 regressed" versus
    // "the suite is red" are the same exit status and completely different
    // instructions.
    let exec = scripted(&[("npm test", Err("FAIL auth::expired"))]);
    let extractor = ScriptedExtractor::new(&[("npm test", &["auth::expired"] as &[&str])]);
    let measured = measure_extracting(
        &exec,
        &ScriptedTriage::none(),
        &extractor,
        None,
        &[gate("unit", "npm test", 600)],
    )
    .await;

    assert_eq!(
        measured[0].run.failing_tests.as_deref(),
        Some(["auth::expired".to_string()].as_slice()),
        "verbatim, so the two sides of the comparison are the same strings"
    );
    assert_eq!(
        extractor.asked(),
        vec!["npm test"],
        "and asked exactly once"
    );
    assert!(
        !measured[0].run.exit_ok,
        "the exit status is still the engine's — the reading says what, never whether"
    );
}

#[tokio::test]
async fn a_green_gate_is_never_handed_to_the_extractor() {
    // A green gate names no failing test, so the answer is knowably empty and
    // asking would make every healthy repository fund the unhealthy case.
    let exec = scripted(&[("npm test", Ok("42 passing"))]);
    let extractor = ScriptedExtractor::new(&[("npm test", &["should-never-be-read"] as &[&str])]);
    let measured = measure_extracting(
        &exec,
        &ScriptedTriage::none(),
        &extractor,
        None,
        &[gate("unit", "npm test", 600)],
    )
    .await;

    assert!(
        extractor.asked().is_empty(),
        "a green gate must cost no agent call: {:?}",
        extractor.asked()
    );
    assert_eq!(
        measured[0].run.failing_tests, None,
        "and it records nothing, which every consumer reads as 'nobody asked'"
    );
}

#[tokio::test]
async fn an_extractor_that_reads_nothing_records_nothing_rather_than_an_empty_list() {
    // The fail-safe. `None` and `Some([])` would compare identically today, but
    // they are different claims — "nobody could read this" versus "the runner
    // named no failing test" — and only the first is true of a spawn failure.
    // Collapsing them onto `None` keeps a malfunctioning extractor
    // indistinguishable from a record written before rung 3 existed.
    let exec = scripted(&[("npm test", Err("Segmentation fault"))]);
    let extractor = ScriptedExtractor::none();
    let measured = measure_extracting(
        &exec,
        &ScriptedTriage::none(),
        &extractor,
        None,
        &[gate("unit", "npm test", 600)],
    )
    .await;

    assert_eq!(extractor.asked(), vec!["npm test"], "it was asked");
    assert_eq!(
        measured[0].run.failing_tests, None,
        "and read nothing, which must not be recorded as an answer"
    );
    assert!(
        !measured[0].run.fingerprint.is_empty(),
        "rungs 1-2 are untouched by a failed reading — that is the whole degradation path"
    );
}

#[tokio::test]
async fn a_red_gate_the_classifier_calls_a_regression_stays_subtractable() {
    // HB2c's own behaviour, preserved. A genuine pre-existing code defect is
    // exactly what decision 44 subtracts; recording a fault here would turn
    // every already-red repository into a terminal failure, which is the
    // opposite of what the baseline is for.
    let exec = scripted(&[("npm test", Err("1 failing\n  auth spec"))]);
    let triage = ScriptedTriage::new(&[("unit", TriageVerdict::Regression)]);
    let measured = measure_with(&exec, &triage, None, &[gate("unit", "npm test", 600)]).await;

    assert!(
        measured[0].run.environment.is_none(),
        "a broken test is a verdict the gate reached — it stays excludable"
    );
    assert_eq!(triage.asked(), vec!["unit"], "and it was asked");
}

#[tokio::test]
async fn a_green_gate_is_never_handed_to_the_classifier() {
    // Cost control, and it is structural rather than a budget check: a healthy
    // repository must not fund the unhealthy case. There is also nothing to
    // classify — a green gate has no failure.
    let exec = scripted(&[
        ("npm run lint", Ok("clean")),
        ("npm test", Ok("42 passing")),
    ]);
    let triage = ScriptedTriage::none();
    let measured = measure_with(
        &exec,
        &triage,
        None,
        &[
            gate("lint", "npm run lint", 600),
            gate("unit", "npm test", 600),
        ],
    )
    .await;

    assert!(
        triage.asked().is_empty(),
        "a green baseline owes no agent call at all: {:?}",
        triage.asked()
    );
    assert!(measured.iter().all(|m| m.run.environment.is_none()));
}

#[tokio::test]
async fn each_red_gate_is_classified_exactly_once() {
    // Once per red gate per measurement — not per validate attempt, which is
    // what reading the answer back off the record buys. Two red gates are two
    // independent questions; one is not evidence about the other.
    let exec = scripted(&[
        ("npm run lint", Err("lint blew up")),
        ("npm test", Err("1 failing")),
        ("npm run e2e", Ok("ok")),
    ]);
    let triage = ScriptedTriage::new(&[("lint", environmental("no browser", "install chromium"))]);
    let measured = measure_with(
        &exec,
        &triage,
        None,
        &[
            gate("lint", "npm run lint", 600),
            gate("unit", "npm test", 600),
            gate("e2e", "npm run e2e", 600),
        ],
    )
    .await;

    assert_eq!(triage.asked(), vec!["lint", "unit"]);
    assert!(measured[0].run.environment.is_some());
    assert!(
        measured[1].run.environment.is_none(),
        "one gate's environmental fault must not spread to another's"
    );
}

#[tokio::test]
async fn a_classifier_that_answers_nothing_useful_leaves_the_gate_subtractable() {
    // The fail-safe direction, stated as a test. `triage_harness_failure`
    // returns `Regression` on every spawn/timeout/cancel/parse failure, so a
    // malfunctioning classifier records no fault — which is the behaviour with
    // no classification at all. A broken classifier must never manufacture a
    // terminal failure.
    let exec = scripted(&[("npm test", Err("1 failing"))]);
    let triage = ScriptedTriage::none();
    let measured = measure_with(&exec, &triage, None, &[gate("unit", "npm test", 600)]).await;

    assert_eq!(measured.len(), 1, "the measurement itself still happened");
    assert!(
        measured[0].run.environment.is_none(),
        "a triage that could not answer withholds the escalation, it does not invent one"
    );
}
