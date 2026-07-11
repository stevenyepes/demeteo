// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::{
    build_environment_message, normalize_failure_fingerprint, parse_triage_text, should_triage,
    TriageVerdict,
};

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

// ── classifier parsing, fail-safe to Regression ─────────────────────────

#[test]
fn parses_environment_verdict() {
    let raw = r#"{"category":"environment","reason":"gdk-3.0 dev package missing","remediation":"install libgtk-3-dev"}"#;
    match parse_triage_text(raw) {
        TriageVerdict::Environment {
            reason,
            remediation,
        } => {
            assert!(reason.contains("gdk-3.0"));
            assert_eq!(remediation, "install libgtk-3-dev");
        }
        _ => panic!("expected environment"),
    }
}

#[test]
fn parses_regression_verdict() {
    let raw = r#"prose... {"category":"regression","reason":"broken test","remediation":""}"#;
    assert_eq!(parse_triage_text(raw), TriageVerdict::Regression);
}

#[test]
fn environment_verdict_amid_prose_and_think_tags() {
    let raw = "<think>maybe env?</think>My verdict:\n{ \"category\": \"environment\", \"reason\": \"no compiler\", \"remediation\": \"install rustc\" }";
    assert!(matches!(
        parse_triage_text(raw),
        TriageVerdict::Environment { .. }
    ));
}

#[test]
fn unparseable_or_unknown_defaults_to_regression() {
    // Fail-safe: a broken/garbage classifier answer must never terminate a
    // real regression — it falls back to the retry path.
    assert_eq!(
        parse_triage_text("I could not decide."),
        TriageVerdict::Regression
    );
    assert_eq!(
        parse_triage_text(r#"{"category":"banana"}"#),
        TriageVerdict::Regression
    );
}

// ── remediation message (C6.3) ──────────────────────────────────────────

#[test]
fn remote_message_has_ssh_reproduce_line_and_context() {
    let msg = build_environment_message(
        "gpu-box",
        "/home/u/wt/feat",
        "cd src-tauri && cargo test",
        "The system library 'gdk-3.0' was not found",
        "install libgtk-3-dev",
    );
    assert!(msg.contains("ssh gpu-box"));
    assert!(msg.contains("cd /home/u/wt/feat && cd src-tauri && cargo test"));
    assert!(msg.contains("Failing command: cd src-tauri && cargo test"));
    assert!(msg.contains("Machine: gpu-box"));
    assert!(msg.contains("install libgtk-3-dev"));
}

#[test]
fn local_message_omits_ssh_line() {
    let msg = build_environment_message(
        "local",
        "/home/u/wt/feat",
        "cargo test",
        "missing lib",
        "install it",
    );
    assert!(!msg.contains("ssh "));
    assert!(msg.contains("cd /home/u/wt/feat && cargo test"));
}
