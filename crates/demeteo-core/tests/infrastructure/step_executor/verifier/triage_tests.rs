// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::{build_environment_message, parse_triage_text, TriageVerdict};

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
