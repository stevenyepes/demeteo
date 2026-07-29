//! Pure decisions over the harness baseline record (HB2a, decision 44).
//!
//! Every test here reaches the policy directly: no port doubles, no
//! `ExecutionDriver`, no I/O. The DB round-trip and the replication path are
//! covered in `tests/infrastructure/database/feature.rs`.

use super::*;

fn run(name: &str, exit_ok: bool, fingerprint: &str) -> HarnessBaselineRun {
    HarnessBaselineRun {
        name: name.to_string(),
        command: format!("npm run {name}"),
        exit_ok,
        fingerprint: fingerprint.to_string(),
        output_ref: Some(format!("/artifacts/{name}.log")),
        measured_at: 1_000,
        producer: BaselineProducer::Node,
    }
}

fn baseline(base_sha: &str, runs: Vec<HarnessBaselineRun>) -> HarnessBaseline {
    HarnessBaseline {
        base_sha: base_sha.to_string(),
        harnesses: runs,
    }
}

// ── The record round-trips ───────────────────────────────────────────────────

#[test]
fn record_round_trips_through_json_unchanged() {
    let record = baseline(
        "abc123",
        vec![run("lint", true, ""), run("unit", false, "fp-unit")],
    );
    let encoded = HarnessBaseline::to_column(Some(&record)).expect("encodes");
    let decoded = HarnessBaseline::from_column(Some(&encoded)).expect("decodes");
    assert_eq!(decoded, record);
}

#[test]
fn base_sha_survives_the_round_trip() {
    let record = baseline("deadbeef", vec![run("unit", false, "fp")]);
    let encoded = HarnessBaseline::to_column(Some(&record)).unwrap();
    assert!(
        encoded.contains("deadbeef"),
        "the sha must be *in* the stored payload: {encoded}"
    );
    let decoded = HarnessBaseline::from_column(Some(&encoded)).unwrap();
    assert_eq!(decoded.base_sha, "deadbeef");
}

#[test]
fn each_gate_keeps_its_own_provenance() {
    let mut fallback = run("unit", false, "fp");
    fallback.producer = BaselineProducer::Fallback;
    fallback.measured_at = 9_999;
    let record = baseline("abc123", vec![run("lint", true, ""), fallback]);

    let decoded =
        HarnessBaseline::from_column(Some(&HarnessBaseline::to_column(Some(&record)).unwrap()))
            .unwrap();
    assert_eq!(
        decoded.harness("lint").unwrap().producer,
        BaselineProducer::Node
    );
    assert_eq!(
        decoded.harness("unit").unwrap().producer,
        BaselineProducer::Fallback
    );
    assert_eq!(decoded.harness("unit").unwrap().measured_at, 9_999);
}

#[test]
fn output_is_a_reference_not_the_output() {
    // Harness output is megabytes; a baseline nobody can afford to read is
    // not a baseline. The record may only carry a pointer.
    let record = baseline("abc123", vec![run("unit", false, "fp")]);
    let encoded = HarnessBaseline::to_column(Some(&record)).unwrap();
    assert!(encoded.contains("/artifacts/unit.log"));
    assert!(
        encoded.len() < 512,
        "the stored payload must stay pointer-sized: {} bytes",
        encoded.len()
    );
}

// ── Absent is not green ──────────────────────────────────────────────────────

#[test]
fn a_missing_column_reads_as_absent() {
    assert!(HarnessBaseline::from_column(None).is_none());
    assert!(HarnessBaseline::from_column(Some("")).is_none());
    assert!(HarnessBaseline::from_column(Some("   ")).is_none());
}

#[test]
fn unreadable_payloads_read_as_absent_not_as_a_green_baseline() {
    // Corrupt JSON, and a record naming a producer this build does not know
    // (a newer schema). Both must decay to "no baseline", because the
    // alternative — inventing a record — would exclude a real regression
    // from validate's verdict.
    assert!(HarnessBaseline::from_column(Some("{not json")).is_none());
    assert!(HarnessBaseline::from_column(Some(
        r#"{"base_sha":"a","harnesses":[{"name":"unit","command":"c","exit_ok":true,"measured_at":1,"producer":"martian"}]}"#
    ))
    .is_none());
}

#[test]
fn an_empty_record_answers_nothing_about_any_gate() {
    // The inversion this whole shape exists to prevent: a record with no
    // measurements must not report a gate as having passed. There is no
    // record-level "was it green" accessor at all, and the per-gate one
    // answers `None` — "never measured".
    let empty = HarnessBaseline::empty("abc123");
    assert!(empty.harness("unit").is_none());
    assert!(empty.harness("lint").is_none());
}

#[test]
fn a_measured_record_answers_only_for_the_gates_it_measured() {
    let record = baseline("abc123", vec![run("lint", true, "")]);
    assert!(record.harness("lint").unwrap().exit_ok);
    assert!(
        record.harness("unit").is_none(),
        "a gate nobody measured must not inherit a sibling's verdict"
    );
}

// ── base_sha is what makes the record evidence ───────────────────────────────

#[test]
fn covers_only_the_sha_that_was_measured() {
    let record = baseline("abc123", vec![run("unit", true, "")]);
    assert!(record.covers("abc123"));
    assert!(!record.covers("def456"));
}

#[test]
fn an_empty_sha_covers_nothing() {
    let record = baseline("", vec![run("unit", true, "")]);
    assert!(!record.covers(""));
}

// ── A partial write merges, not clobbers ─────────────────────────────────────

#[test]
fn merging_into_nothing_stores_the_incoming_record() {
    let incoming = baseline("abc123", vec![run("unit", true, "")]);
    assert_eq!(HarnessBaseline::merge(None, incoming.clone()), incoming);
}

#[test]
fn a_partial_write_keeps_the_gates_it_did_not_measure() {
    // HB2b's fallback measures the one gate that just went red. The node
    // measured all three. Replacing would silently narrow the subtraction
    // to one harness.
    let stored = baseline(
        "abc123",
        vec![
            run("lint", true, ""),
            run("unit", true, ""),
            run("integration", false, "fp-int"),
        ],
    );
    let mut remeasured = run("unit", false, "fp-unit");
    remeasured.producer = BaselineProducer::Fallback;
    remeasured.measured_at = 5_000;

    let merged = HarnessBaseline::merge(Some(stored), baseline("abc123", vec![remeasured]));

    assert_eq!(merged.harnesses.len(), 3, "no gate may be dropped");
    assert!(merged.harness("lint").unwrap().exit_ok);
    assert_eq!(merged.harness("integration").unwrap().fingerprint, "fp-int");
    let unit = merged.harness("unit").unwrap();
    assert!(!unit.exit_ok, "the re-measurement must win");
    assert_eq!(unit.fingerprint, "fp-unit");
    assert_eq!(unit.producer, BaselineProducer::Fallback);
}

#[test]
fn a_remeasured_gate_keeps_its_position() {
    // Declared gate order is cheap-first and HB7 renders it; an upsert that
    // moved the re-measured gate to the end would reorder the report.
    let stored = baseline("abc123", vec![run("lint", true, ""), run("unit", true, "")]);
    let merged = HarnessBaseline::merge(
        Some(stored),
        baseline("abc123", vec![run("lint", false, "fp")]),
    );
    let names: Vec<&str> = merged.harnesses.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["lint", "unit"]);
}

#[test]
fn a_new_gate_is_appended() {
    let stored = baseline("abc123", vec![run("lint", true, "")]);
    let merged = HarnessBaseline::merge(
        Some(stored),
        baseline("abc123", vec![run("unit", false, "fp")]),
    );
    let names: Vec<&str> = merged.harnesses.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["lint", "unit"]);
}

#[test]
fn a_different_base_sha_replaces_rather_than_blends() {
    // The stored entries describe another commit. Keeping them under the
    // new sha would produce a record whose own `base_sha` is false for half
    // its contents — worse than having no baseline.
    let stored = baseline("abc123", vec![run("lint", true, ""), run("unit", true, "")]);
    let merged = HarnessBaseline::merge(
        Some(stored),
        baseline("def456", vec![run("unit", false, "fp")]),
    );
    assert_eq!(merged.base_sha, "def456");
    assert_eq!(merged.harnesses.len(), 1);
    assert!(
        merged.harness("lint").is_none(),
        "a measurement of other code must not survive the rebase"
    );
}

// ── fallback_baseline_needed (HB2b) ──────────────────────────────────────────
//
// The lazy fallback is the expensive producer: `prepare_command` plus a suite
// against a cold checkout, i.e. minutes. Every test here is about *not* paying
// that unless the answer it buys is worth it.

fn gates(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_green_harness_never_measures_a_fallback_baseline() {
    // The load-bearing one. Nothing failed, so there is nothing to subtract
    // from — and firing here would add minutes to every *successful* run
    // forever, to answer a question nobody asked.
    assert!(!fallback_baseline_needed(
        false,
        "abc123",
        None,
        &gates(&["unit"])
    ));
}

#[test]
fn a_red_harness_with_no_record_measures() {
    assert!(fallback_baseline_needed(
        true,
        "abc123",
        None,
        &gates(&["unit"])
    ));
}

#[test]
fn a_covering_record_holding_every_failing_gate_does_not_re_measure() {
    // The cache. A second validate attempt in the same run must not pay for
    // the same measurement twice — which is the whole reason the fallback
    // persists what it measured.
    let stored = baseline("abc123", vec![run("unit", false, "fp")]);
    assert!(!fallback_baseline_needed(
        true,
        "abc123",
        Some(&stored),
        &gates(&["unit"])
    ));
}

#[test]
fn a_green_record_also_counts_as_measured() {
    // "Was it green?" is not one of the inputs: a stored measurement answers
    // the question either way, and re-running a gate at a commit it was
    // already measured at cannot produce new information.
    let stored = baseline("abc123", vec![run("unit", true, "")]);
    assert!(!fallback_baseline_needed(
        true,
        "abc123",
        Some(&stored),
        &gates(&["unit"])
    ));
}

#[test]
fn a_record_for_a_different_commit_does_not_count() {
    // It describes other code. Trusting it is exactly the mistake `base_sha`
    // exists to make detectable.
    let stored = baseline("def456", vec![run("unit", false, "fp")]);
    assert!(fallback_baseline_needed(
        true,
        "abc123",
        Some(&stored),
        &gates(&["unit"])
    ));
}

#[test]
fn a_record_missing_one_of_the_failing_gates_measures() {
    // Partial coverage is not coverage: `lint` has no baseline, so the gate
    // that just went red would get no subtraction.
    let stored = baseline("abc123", vec![run("unit", false, "fp")]);
    assert!(fallback_baseline_needed(
        true,
        "abc123",
        Some(&stored),
        &gates(&["unit", "lint"])
    ));
}

#[test]
fn an_unresolvable_base_never_measures() {
    // A measurement that cannot name its commit is not evidence. No baseline
    // is the honest answer; a baseline against an unknown commit is not.
    for sha in ["", "   "] {
        assert!(!fallback_baseline_needed(
            true,
            sha,
            None,
            &gates(&["unit"])
        ));
    }
}

#[test]
fn no_gates_never_measures() {
    assert!(!fallback_baseline_needed(true, "abc123", None, &[]));
}

// ── The `{{harness_baseline}}` briefing (HB2c) ───────────────────────────────

fn gate(name: &str, command: &str) -> crate::domain::verifier::ResolvedHarness {
    crate::domain::verifier::ResolvedHarness {
        name: name.to_string(),
        command: command.to_string(),
        deadline_s: 600,
    }
}

/// The positive statement the spec prompt never had. Both failed validate
/// attempts in `f-1785157902856` cost a rework cycle because the acceptance
/// criteria named commands the harness never ran — so the block has to name
/// every gate *and* its command, or the reader is guessing again.
#[test]
fn the_briefing_names_every_gate_and_the_command_it_runs() {
    let gates = [gate("lint", "npm run lint"), gate("unit", "cargo test")];
    let rendered = render_harness_briefing(&gates, None);

    for (name, cmd) in [("lint", "npm run lint"), ("unit", "cargo test")] {
        assert!(
            rendered.contains(name) && rendered.contains(cmd),
            "the briefing must name {name} and {cmd}; got:\n{rendered}"
        );
    }
    assert!(
        rendered.contains("only"),
        "it must say these are the only commands executed, or a criterion \
         against some other command still looks provable; got:\n{rendered}"
    );
}

/// The `not_configured` row, reached at spec time instead of after the whole
/// implement budget. It is not enough to omit the command list: a spec author
/// told nothing will run has to be told what that *means* for a criterion, and
/// where the setting that would change it lives.
#[test]
fn no_gates_at_all_says_so_and_says_what_it_costs() {
    let rendered = render_harness_briefing(&[], None);

    assert!(
        rendered.contains("NOTHING"),
        "an unconfigured harness must be unmissable; got:\n{rendered}"
    );
    assert!(
        rendered.contains("never be shown MET"),
        "the consequence for a criterion is the whole point of saying it early; \
         got:\n{rendered}"
    );
    assert!(
        rendered.contains("Open Question"),
        "and it must say where to record it; got:\n{rendered}"
    );
    assert!(
        rendered.contains("project setting"),
        "no amount of re-implementation fixes a missing command — the block must \
         say which lever does; got:\n{rendered}"
    );
}

/// A gate that was already failing is *not* a gate whose output can evidence a
/// new criterion. Saying only "these commands run" would let a spec author
/// write a criterion against a suite that has never been green here.
#[test]
fn a_gate_already_red_at_the_base_is_flagged_as_such() {
    let record = baseline("abc123", vec![run("unit", false, "fp")]);
    let rendered = render_harness_briefing(&[gate("unit", "npm run unit")], Some(&record));

    assert!(
        rendered.contains("already failing"),
        "a red baseline must be stated, not left to be inferred; got:\n{rendered}"
    );
}

#[test]
fn a_gate_green_at_the_base_says_it_passed() {
    let record = baseline("abc123", vec![run("unit", true, "")]);
    let rendered = render_harness_briefing(&[gate("unit", "npm run unit")], Some(&record));

    assert!(rendered.contains("passed"), "got:\n{rendered}");
    assert!(!rendered.contains("already failing"), "got:\n{rendered}");
}

/// Absent is not green, in the prompt as well as in the verdict. A gate with no
/// measurement must not render as a blank line the reader completes as "fine".
#[test]
fn an_unmeasured_gate_is_reported_as_unknown_not_as_passing() {
    let record = baseline("abc123", vec![run("lint", true, "")]);
    let rendered = render_harness_briefing(&[gate("unit", "npm run unit")], Some(&record));

    assert!(
        rendered.contains("not measured"),
        "silence about a gate must be stated as silence; got:\n{rendered}"
    );
    assert!(
        !rendered.contains("passed"),
        "an unmeasured gate must not borrow another gate's green; got:\n{rendered}"
    );
}
