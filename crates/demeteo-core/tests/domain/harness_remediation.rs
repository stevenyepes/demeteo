// Tests for `src/domain/harness_remediation.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::{build_environment_message, build_timeout_message};

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

// ── the ceiling message (HB3 shrank the population that reaches it) ─────

#[test]
fn a_timeout_leads_with_watch_mode_and_names_the_ceiling() {
    let msg = build_timeout_message("local", "/home/u/wt/feat", "npm test", 900);

    // The ceiling has to be in the message: "it was slow" and "it was
    // abandoned at 900s" are different claims, and only the second tells the
    // user which preference to raise.
    assert!(msg.contains("900s"), "got:\n{msg}");
    // Watch mode is the overwhelming cause, so it leads the remediation rather
    // than sitting under a list of possibilities.
    let remediation = msg
        .split("Remediation: ")
        .nth(1)
        .expect("a timeout carries remediation");
    assert!(
        remediation.starts_with("The usual cause is a test runner left in **watch mode**"),
        "got:\n{remediation}"
    );
    assert!(msg.contains("Failing command: npm test"));
}
