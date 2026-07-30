// Tests extracted from `src/adapters/step_executor/driver/verifier.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::build_environment_message;

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
