// Tests extracted from `src/adapters/step_executor/driver/verifier.rs`, moved
// with the code to `src/domain/harness_failure.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::detect_missing_command;

// ── the three shell diagnostics we can actually receive ──────────────────

#[test]
fn detects_dash_not_found() {
    // The exact failure from a detached run on Ubuntu: `/bin/sh` is dash, and
    // the harness ran before this fix under a bare non-login shell.
    let out = "Command failed (exit code: Some(127)): sh: 1: cargo: not found\n";
    assert_eq!(
        detect_missing_command("cd src-tauri && cargo test", out).as_deref(),
        Some("cargo")
    );
}

#[test]
fn detects_bash_command_not_found() {
    let out = "bash: line 1: pytest: command not found\n";
    assert_eq!(
        detect_missing_command("pytest -q", out).as_deref(),
        Some("pytest")
    );
}

#[test]
fn detects_zsh_command_not_found() {
    // zsh puts the name *after* the message rather than before it.
    let out = "zsh: command not found: npm\n";
    assert_eq!(
        detect_missing_command("npm test", out).as_deref(),
        Some("npm")
    );
}

#[test]
fn detects_ubuntu_command_not_found_handler() {
    // The default on Ubuntu: `command-not-found` is wired into bash's
    // `command_not_found_handle` hook and *replaces* the shell's own
    // diagnostic, so none of the three classic strings ever appear. This is the
    // shape the detached run on 10.27.40.55 actually produced.
    let out = "Command 'cargo' not found, but can be installed with:\n\
               sudo apt install cargo   # version 1.75.0+dfsg0ubuntu1-0ubuntu7.4, or\n\
               sudo apt install rustup  # version 1.26.0-5ubuntu0.1\n";
    assert_eq!(
        detect_missing_command("cd src-tauri && cargo test", out).as_deref(),
        Some("cargo")
    );
}

#[test]
fn detects_older_ubuntu_no_command_found_wording() {
    let out = "No command 'pytest' found, did you mean:\n Command 'pytest-3' from package 'python3-pytest'\n";
    assert_eq!(
        detect_missing_command("pytest -q", out).as_deref(),
        Some("pytest")
    );
}

#[test]
fn ignores_quoted_not_found_for_a_command_the_harness_never_runs() {
    // The false-positive guard covers the quoted shape too: `apt`'s suggestion
    // lines name *other* binaries, and a test that prints the handler's wording
    // must stay a Verdict.
    let out = "Command 'rustc' not found, but can be installed with:\n";
    assert_eq!(detect_missing_command("npm test", out), None);
}

#[test]
fn detects_missing_command_from_ssh_adapter_error_shape() {
    // The SSH adapter substitutes remote stderr for the exit code whenever
    // stderr is non-empty, so the string carries no "127" at all — the
    // detector must key off the shell diagnostic, not the code.
    let out = "Command failed (sh: 1: cargo: not found): bash -l -i -c 'cargo test'";
    assert_eq!(
        detect_missing_command("cargo test", out).as_deref(),
        Some("cargo")
    );
}

#[test]
fn detects_zsh_wrapped_in_the_ssh_adapter_error_shape() {
    // zsh names the binary last, so the adapter's own `): …` suffix ends up
    // glued to it — the name must still come out clean.
    let out = "Command failed (zsh: command not found: npm): bash -l -i -c 'npm test'";
    assert_eq!(
        detect_missing_command("npm test", out).as_deref(),
        Some("npm")
    );
}

// ── the false-positive guard: the name must be one the command invokes ───

#[test]
fn ignores_not_found_printed_by_a_test_the_harness_ran() {
    // A green-path build whose *test output* merely contains the string must
    // stay a normal Verdict — escalating here would terminate a real
    // regression with no retries. `foo` is nowhere in the harness command.
    let out = "test cli::reports_missing_binary ... FAILED\n\
               assertion failed: expected `sh: 1: foo: not found`\n";
    assert_eq!(detect_missing_command("cargo test", out), None);
}

#[test]
fn ignores_indirect_invocation_not_named_in_the_command() {
    // `make test` shelling out to a missing `cargo` does not match — by
    // design. It falls through to the existing triage path, which reaches the
    // same conclusion one attempt later. Documented, not accidental.
    let out = "sh: 1: cargo: not found\nmake: *** [test] Error 127\n";
    assert_eq!(detect_missing_command("make test", out), None);
}

#[test]
fn ignores_ordinary_red_build() {
    let out = "error[E0308]: mismatched types\n --> src/main.rs:4:5\n";
    assert_eq!(detect_missing_command("cargo test", out), None);
}

#[test]
fn matches_command_token_only_on_a_word_boundary() {
    // A substring of a token must not count as an invocation: the harness runs
    // `cargo-nextest`, and a stray `cargo: not found` line should not match it.
    let out = "sh: 1: cargo: not found\n";
    assert_eq!(detect_missing_command("cargo-nextest run", out), None);
}
