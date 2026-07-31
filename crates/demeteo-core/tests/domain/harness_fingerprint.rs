// Tests extracted from `src/adapters/step_executor/driver/verifier.rs`, moved
// with the code to `src/domain/harness_fingerprint.rs` (mirrored-tests
// convention). `super` resolves to that module.

use super::{normalize_failure_fingerprint, should_triage};

// ── fingerprint normalization: the load-bearing part (C6.2) ──────────────

#[test]
fn same_failure_differing_only_in_worktree_and_timestamp_fingerprint_matches() {
    // Two attempts of the *same* missing-lib failure whose logs differ only
    // in the per-run worktree path and an epoch-ms timestamp MUST
    // fingerprint-match, so triage actually fires (the under-normalization
    // guard the DoD's differing-output fixture does not catch).
    let wt1 = "/home/u/.demeteo/wt/se-feat-s-impl-1699999999999";
    let wt2 = "/home/u/.demeteo/wt/se-feat-s-impl-1700000000000";
    let log = |wt: &str, ts: &str| {
        format!(
            "error: The system library 'gdk-3.0' was not found\n  building {}/build.rs\n  \
             at epoch {}\n",
            wt, ts
        )
    };
    let a = normalize_failure_fingerprint(&log(wt1, "1699999999999"), wt1);
    let b = normalize_failure_fingerprint(&log(wt2, "1700000000000"), wt2);
    assert_eq!(a, b, "volatile-only differences must fingerprint-match");
}

#[test]
fn npm_failure_fingerprints_identically_across_attempts() {
    // The `f-1d0209a0e43d5b67` incident, verbatim. Every npm failure ends with
    // a debug-log path whose timestamp is the *only* thing that moves between
    // attempts — and whose longest digit run is `2026`, well under
    // `mask_long_digit_runs`'s six. Before timestamp masking the fingerprint
    // therefore differed every time, `should_triage` never returned true, and
    // the C6 classifier was never consulted on any npm-based project.
    let wt = "/home/developer/wt/se-feat-s-validate";
    let attempt = |stamp: &str| {
        format!(
            "Command failed (exit code: Some(1)): npm error Missing script: \"checks:code\"\n\
             npm error\n\
             npm error To see a list of scripts, run:\n\
             npm error   npm run\n\
             npm error A complete log of this run can be found in: \
             /home/developer/.npm/_logs/{stamp}-debug-0.log\n\
             \n\
             bash: cannot set terminal process group (-1): Inappropriate ioctl for device\n\
             bash: no job control in this shell\n"
        )
    };
    let a = normalize_failure_fingerprint(&attempt("2026-07-30T17_39_51_520Z"), wt);
    let b = normalize_failure_fingerprint(&attempt("2026-07-30T18_04_12_007Z"), wt);
    assert_eq!(a, b, "the same npm failure must fingerprint identically");
    assert!(should_triage(Some(&a), &b));
}

#[test]
fn npm_debug_log_sequence_number_does_not_perturb_the_fingerprint() {
    // npm bumps the trailing index when two runs land on the same millisecond.
    let wt = "";
    let line = |n: u32| {
        format!("npm error log: /home/u/.npm/_logs/2026-07-30T17_39_51_520Z-debug-{n}.log\n")
    };
    assert_eq!(
        normalize_failure_fingerprint(&line(0), wt),
        normalize_failure_fingerprint(&line(3), wt)
    );
}

#[test]
fn colon_separated_iso_timestamps_are_masked_too() {
    // The same instant as npm's filename form, written the way a log line does.
    let wt = "";
    let line = |ts: &str| format!("[{ts}] ERROR pipeline stalled\n");
    assert_eq!(
        normalize_failure_fingerprint(&line("2026-07-30T17:39:51.520Z"), wt),
        normalize_failure_fingerprint(&line("2026-07-31T04:02:00.001Z"), wt)
    );
    assert_eq!(
        normalize_failure_fingerprint(&line("2026-07-30 17:39:51"), wt),
        normalize_failure_fingerprint(&line("2026-07-31 04:02:00"), wt)
    );
}

#[test]
fn timestamp_masking_does_not_collapse_different_compiler_errors() {
    // The over-masking guard for the new mask specifically: two genuinely
    // different failures that both carry a volatile npm log line must still
    // fingerprint apart, or a real regression would be terminated as
    // "environment".
    let wt = "";
    let with_log = |err: &str, stamp: &str| {
        format!(
            "{err}\nnpm error A complete log of this run can be found in: \
             /home/u/.npm/_logs/{stamp}-debug-0.log\n"
        )
    };
    let a = normalize_failure_fingerprint(
        &with_log("error[E0308]: mismatched types", "2026-07-30T17_39_51_520Z"),
        wt,
    );
    let b = normalize_failure_fingerprint(
        &with_log(
            "error[E0425]: cannot find value `x`",
            "2026-07-30T18_04_12_007Z",
        ),
        wt,
    );
    assert_ne!(a, b);
    assert!(!should_triage(Some(&a), &b));
}

#[test]
fn a_bare_date_or_clock_time_is_not_mistaken_for_a_timestamp() {
    // The mask is anchored on a full date *immediately* followed by a time, so
    // dates and durations a failure message is actually made of survive.
    let wt = "";
    let kept = normalize_failure_fingerprint(
        "expected release 2026-07-30, got 2026-07-31\ntest result: FAILED in 12:30\n",
        wt,
    );
    assert!(kept.contains("2026-07-30"));
    assert!(kept.contains("2026-07-31"));
    assert!(kept.contains("12:30"));
}

#[test]
fn genuinely_different_errors_fingerprint_differently() {
    // Over-normalization guard: a different regression error on the retry
    // must NOT read as "same" (or we'd triage real progress).
    let wt = "/tmp/wt";
    let a = normalize_failure_fingerprint("error[E0308]: mismatched types in auth.rs\n", wt);
    let b = normalize_failure_fingerprint("error: test payments::refund panicked\n", wt);
    assert_ne!(a, b);
}

#[test]
fn short_numbers_and_versions_are_preserved() {
    // We must NOT mask line numbers / exit codes / version components, or
    // distinct failures would collapse together.
    let wt = "";
    let a = normalize_failure_fingerprint("gdk-3.0 not found (exit 1) at line 42\n", wt);
    assert!(a.contains("gdk-3.0"));
    assert!(a.contains("exit 1"));
    assert!(a.contains("line 42"));
}

// ── the persistence gate (C6.2) ─────────────────────────────────────────

#[test]
fn first_failure_does_not_trigger_triage() {
    assert!(!should_triage(None, "fp"));
}

#[test]
fn changed_failure_does_not_trigger_triage() {
    assert!(!should_triage(Some("old"), "new"));
}

#[test]
fn reproduced_failure_triggers_triage() {
    assert!(should_triage(Some("same"), "same"));
}
