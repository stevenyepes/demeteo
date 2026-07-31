// Tests extracted from `src/adapters/step_executor/driver/verifier.rs`, moved
// with the code to `src/domain/harness_failure.rs` (mirrored-tests convention).
// `super` resolves to that module.

use super::{build_missing_task_message, detect_missing_task, MissingTask};

fn detected(cmd: &str, out: &str) -> Option<(String, String)> {
    detect_missing_task(cmd, out).map(|m| (m.runner.to_string(), m.name.clone()))
}

// ── the incident: npm ≥ 9, wrapped by the execution adapter ──────────────

/// Verbatim from feature `f-1d0209a0e43d5b67`, which burned 7 validate and ~20
/// implement attempts on it. Exit code 1, no shell diagnostic, so the 127 fast
/// path could not see it.
const NPM_INCIDENT: &str = "Command failed (exit code: Some(1)): npm error Missing script: \"checks:code\"\n\
     npm error\n\
     npm error To see a list of scripts, run:\n\
     npm error   npm run\n\
     npm error A complete log of this run can be found in: /home/developer/.npm/_logs/2026-07-30T17_39_51_520Z-debug-0.log\n\
     \n\
     bash: cannot set terminal process group (-1): Inappropriate ioctl for device\n\
     bash: no job control in this shell\n";

#[test]
fn detects_npm_missing_script_from_the_incident_output() {
    assert_eq!(
        detected("npm run checks:code", NPM_INCIDENT),
        Some(("npm".to_string(), "checks:code".to_string()))
    );
}

#[test]
fn detects_older_npm_err_bang_wording() {
    // npm 7–8 kept the quotes but used the `ERR!` severity marker.
    let out = "npm ERR! Missing script: \"checks:code\"\nnpm ERR! A complete log …\n";
    assert_eq!(
        detected("npm run checks:code", out),
        Some(("npm".to_string(), "checks:code".to_string()))
    );
}

#[test]
fn detects_npm_6_lowercase_unquoted_wording() {
    let out = "npm ERR! missing script: checks:code\n";
    assert_eq!(
        detected("npm run checks:code", out),
        Some(("npm".to_string(), "checks:code".to_string()))
    );
}

#[test]
fn detects_pnpm_missing_script() {
    let out = " ERR_PNPM_NO_SCRIPT  Missing script: checks:code\n";
    assert_eq!(
        detected("pnpm run checks:code", out),
        Some(("pnpm".to_string(), "checks:code".to_string()))
    );
}

#[test]
fn detects_yarn_command_not_found() {
    // yarn 1 does not say "script" at all — it reports the *command* it was
    // asked to run, which reads like a missing binary but is not one.
    let out = "error Command \"checks:code\" not found.\n";
    assert_eq!(
        detected("yarn run checks:code", out),
        Some(("yarn".to_string(), "checks:code".to_string()))
    );
}

#[test]
fn detects_make_missing_target() {
    let out = "make: *** No rule to make target 'checks'.  Stop.\n";
    assert_eq!(
        detected("make checks", out),
        Some(("make".to_string(), "checks".to_string()))
    );
}

#[test]
fn detects_make_historical_backquote_form() {
    // GNU make used to open with a backquote and close with an apostrophe.
    let out = "make[1]: *** No rule to make target `checks'.  Stop.\n";
    assert_eq!(
        detected("make -j4 checks", out),
        Some(("make".to_string(), "checks".to_string()))
    );
}

// ── the false-positive guard: terminating is irreversible ────────────────

#[test]
fn ignores_missing_script_printed_by_a_test_the_harness_ran() {
    // The required guard: a suite whose *own output* quotes npm's wording must
    // stay a normal Verdict. Escalating here would terminate a real regression
    // with no retries left.
    let out = "test cli::reports_missing_script ... FAILED\n\
               assertion failed: expected 'Missing script: \"checks:code\"'\n";
    assert_eq!(detected("npm run checks:code", out), None);
}

#[test]
fn ignores_missing_script_when_the_harness_runs_no_task_runner() {
    // Same words, but `cargo test` invokes none of npm/pnpm/yarn/bun, so
    // nothing here could have reported them.
    let out = "npm error Missing script: \"checks:code\"\n";
    assert_eq!(detected("cargo test", out), None);
}

#[test]
fn ignores_a_script_name_the_harness_command_never_asks_for() {
    // A nested `npm run` inside the project's own test suite failing on some
    // *other* script is a red build, not a misconfigured command setting.
    let out = "npm error Missing script: \"docs:build\"\n";
    assert_eq!(detected("npm run checks:code", out), None);
}

#[test]
fn ignores_make_target_the_command_never_names() {
    // The documented cost of the token guard: a prerequisite named only inside
    // the Makefile falls through to the ordinary triage path.
    let out = "make: *** No rule to make target 'generated.rs', needed by 'checks'.  Stop.\n";
    assert_eq!(detected("make checks", out), None);
}

#[test]
fn ignores_an_ordinary_red_build() {
    let out = "error[E0308]: mismatched types\n --> src/main.rs:4:5\n";
    assert_eq!(detected("npm run checks:code", out), None);
}

#[test]
fn ignores_the_ubuntu_missing_binary_handler() {
    // That shape belongs to the 127 path, which runs first and owns the
    // "install it" remediation. This detector must not claim it and mislabel a
    // provisioning gap as a bad command setting.
    let out = "Command 'npm' not found, but can be installed with:\nsudo apt install npm\n";
    assert_eq!(detected("npm run checks:code", out), None);
}

// ── the remediation must point at the setting, not at a package manager ──

#[test]
fn remediation_names_the_project_command_setting_and_the_base_commit() {
    let msg = build_missing_task_message(
        "gpu-box",
        "/home/u/wt/feat",
        "npm run checks:code",
        &MissingTask {
            runner: "npm",
            name: "checks:code".to_string(),
        },
    );
    // The actual root cause of the incident: the project's configured command
    // and the worktree's base commit disagreed.
    assert!(msg.contains("project's configured prepare/test command"));
    assert!(msg.contains("base commit"));
    assert!(msg.contains("checks:code"));
    // …and it must *not* send the user after a package that was never missing.
    assert!(msg.contains("Nothing needs installing"));
    assert!(!msg.contains("PATH"));
    // The shared context block still applies.
    assert!(msg.contains("ssh gpu-box"));
    assert!(msg.contains("npm run"));
}

#[test]
fn make_remediation_says_target_not_script() {
    let msg = build_missing_task_message(
        "local",
        "/home/u/wt/feat",
        "make checks",
        &MissingTask {
            runner: "make",
            name: "checks".to_string(),
        },
    );
    assert!(msg.contains("target named `checks`"));
    assert!(!msg.contains("script named"));
    assert!(msg.contains("Makefile"));
}
