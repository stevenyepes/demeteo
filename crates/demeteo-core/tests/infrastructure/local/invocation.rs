//! What gets started, and how it is named — the part decided before any
//! process exists.
//!
//! No child is spawned here on purpose: every assertion is about a decision,
//! and the Windows half of each is reachable from the macOS and Linux hosts
//! this is developed on precisely because the decision is not behind a `cfg`.
//! The adapter-level counterparts, which do spawn, are in `execution.rs`
//! beside them.

use super::*;
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::ports::execution::{ShellOptions, TRANSPORT_ERROR_PREFIX};
use crate::shared::win::posix_shell::ShellMissing;

// ── what the shell is invoked as ─────────────────────────────────────────────
//
// `shell_args` carries no `#[cfg]` and these tests carry none either: the body
// and its argv are the part that must be byte-identical on all three desktop
// targets and on the always-Linux runner, so a platform-conditional assertion
// about them would be asserting the wrong thing. Only the program half varies,
// and only its Unix answer is assertable from here — no Windows toolchain runs
// on the development host.

#[test]
fn an_interactive_login_shell_is_l_i_c_with_job_control_off_and_env_inside_the_body() {
    let mut opts = ShellOptions::login_interactive();
    opts.env.insert("TOKEN".to_string(), "s'quote".to_string());

    assert_eq!(
        shell_args("npm test", &opts),
        vec![
            "-l",
            "-i",
            "-c",
            "set +m; export TOKEN='s'\\''quote'; npm test"
        ]
    );
}

#[test]
fn a_non_interactive_login_shell_drops_the_i_and_the_job_control_prefix() {
    assert_eq!(
        shell_args("npm test", &ShellOptions::login()),
        vec!["-l", "-c", "npm test"]
    );
}

#[test]
fn a_plain_shell_is_c_and_the_body_alone() {
    assert_eq!(
        shell_args("npm test", &ShellOptions::default()),
        vec!["-c", "npm test"]
    );
}

#[test]
fn the_working_directory_never_reaches_the_body() {
    // `current_dir` carries it instead. A Windows path in the body would be a
    // string of escape sequences to bash, and the SSH adapter — which has no
    // such channel — is the reason the two constructions are shared at all.
    let opts = ShellOptions {
        cwd: Some(r"C:\work\demeteo".to_string()),
        ..ShellOptions::login_interactive()
    };
    assert_eq!(
        shell_args("npm test", &opts),
        vec!["-l", "-i", "-c", "set +m; npm test"]
    );
}

#[cfg(unix)]
#[test]
fn on_unix_the_program_is_the_bare_name_execvp_resolves() {
    let (program, args) = shell_invocation("npm test", &ShellOptions::login_interactive())
        .expect("a Unix host always has a shell to name");
    assert_eq!(program, PathBuf::from("bash"));
    assert_eq!(args, vec!["-l", "-i", "-c", "set +m; npm test"]);

    let (program, args) = shell_invocation("npm test", &ShellOptions::default())
        .expect("a Unix host always has a shell to name");
    assert_eq!(program, PathBuf::from("sh"));
    assert_eq!(args, vec!["-c", "npm test"]);
}

#[test]
fn an_unresolvable_shell_reads_as_a_transport_failure_never_as_an_exit_code() {
    // D3: the command did not run, so it has no verdict. Anything unprefixed
    // here would be read as the project's own command having failed, and the
    // rework loop would hand an agent code to fix that was never executed.
    let err = no_posix_shell_error(&ShellMissing::NoGitForWindows { searched: vec![] });

    assert!(err.starts_with(TRANSPORT_ERROR_PREFIX), "got: {err}");
    assert!(
        err[TRANSPORT_ERROR_PREFIX.len()..].starts_with(NO_POSIX_SHELL_ERROR),
        "the preflight matches on this position; got: {err}"
    );
    assert!(
        err.contains("no Git for Windows"),
        "which of the several failures happened must survive: {err}"
    );
}

/// The Windows arm, decided on a host that has no Windows. Git's
/// `mingw_access` masks `X_OK` off, so a hook with no bit to test is still a
/// hook Git will attempt — answering `false` here would silently stop a
/// repository's `commit-msg` from ever vetting a message Demeteo wrote.
#[test]
fn a_hook_with_no_permission_bits_to_read_is_still_run() {
    assert!(git_would_run_hook(false, None));
    assert!(!git_would_run_hook(true, None), "a directory is not a hook");
}

#[test]
fn a_hook_carrying_permission_bits_is_run_only_when_one_of_them_is_executable() {
    assert!(git_would_run_hook(false, Some(0o755)));
    assert!(git_would_run_hook(false, Some(0o100)), "owner-only counts");
    assert!(!git_would_run_hook(false, Some(0o644)));
    assert!(!git_would_run_hook(true, Some(0o755)));
}

#[test]
fn an_ordinary_spawn_failure_is_left_to_read_as_it_always_did() {
    let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
    assert!(unspawnable_arguments("npm", &missing).is_none());
}

#[test]
fn an_argument_no_spawn_can_carry_is_a_configuration_error_not_a_verdict() {
    let refused = std::io::Error::new(std::io::ErrorKind::InvalidInput, "cannot be escaped");
    let err = unspawnable_arguments("opencode", &refused).expect("InvalidInput is that error");

    assert_eq!(
        classify_exec_failure(&err),
        HarnessExecFailure::Transport,
        "anything else feeds the rework loop a failure no ticket can close: {err}"
    );
    assert!(
        err[TRANSPORT_ERROR_PREFIX.len()..].starts_with(UNSPAWNABLE_ARGUMENTS_ERROR),
        "a matcher reads this at a fixed position, as it does the missing-shell error: {err}"
    );
    assert!(err.contains("opencode"), "got: {err}");
}
