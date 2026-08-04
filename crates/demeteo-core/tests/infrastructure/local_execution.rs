//! `LocalSubprocessAdapter::run_command_with` — the deadline and the kill.
//!
//! These spawn **real** processes, on purpose. The bug they exist for was
//! invisible to a mocked port: the adapter used to run `Command::output()`
//! inside `spawn_blocking`, so a `tokio::time::timeout` around the returned
//! future abandoned the *wait* while the child kept running — holding open a
//! worktree the driver was about to delete, and occupying a blocking-pool
//! thread for as long as the command felt like taking. Only an actual child
//! process can tell you whether it died.

use super::*;
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::ports::execution::{ProgramRequest, TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
use crate::shared::win::posix_shell::ShellMissing;
use std::time::{Duration, Instant};

/// `interactive` is what makes the child a session leader (`setsid`), which is
/// what makes the process-group kill safe — and it is what every `command`
/// node uses, via `harness_shell_options`.
///
/// Every caller is a `#[cfg(unix)]` test that spawns a real shell, so on
/// Windows this has none.
#[cfg(unix)]
fn harness_opts(timeout: Option<Duration>) -> ShellOptions {
    ShellOptions {
        timeout,
        ..ShellOptions::login_interactive()
    }
}

/// Is `pid` still alive? `kill(pid, 0)` is the standard liveness probe.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(unix)]
#[tokio::test]
async fn a_command_that_finishes_returns_its_stdout() {
    let adapter = LocalSubprocessAdapter::new();
    let out = adapter
        .run_command_with("local", "echo hello", harness_opts(None))
        .await
        .expect("echo succeeds");
    assert_eq!(out.trim(), "hello");
}

#[cfg(unix)]
#[tokio::test]
async fn a_program_request_preserves_argv_cwd_and_environment() {
    let adapter = LocalSubprocessAdapter::new();
    let cwd = std::env::temp_dir();
    let mut env = std::collections::BTreeMap::new();
    env.insert("DEMETEO_ARGV_TEST".to_string(), "present".to_string());
    let out = adapter
        .run_program(
            "local",
            ProgramRequest {
                executable: "sh".to_string(),
                args: vec![
                    "-c".to_string(),
                    "printf '%s|%s|%s' \"$1\" \"$DEMETEO_ARGV_TEST\" \"$PWD\"".to_string(),
                    "ignored".to_string(),
                    "value with spaces".to_string(),
                ],
                cwd: Some(cwd.to_string_lossy().into_owned()),
                env,
                timeout: Some(Duration::from_secs(5)),
            },
        )
        .await
        .expect("structured argv request succeeds");
    assert!(out.starts_with("value with spaces|present|"), "got: {out}");
}

#[cfg(windows)]
#[tokio::test]
async fn a_windows_program_request_preserves_argv_cwd_and_environment_with_spaces() {
    let adapter = LocalSubprocessAdapter::new();
    let cwd = std::env::temp_dir().join(format!("demeteo argv spaces {}", std::process::id()));
    std::fs::create_dir_all(&cwd).expect("scratch directory");
    let script = cwd.join("inspect.ps1");
    std::fs::write(
        &script,
        "param([string]$Value) [Console]::Write(\"$Value|$env:DEMETEO_ARGV_TEST|$PWD\")",
    )
    .expect("script");
    let mut env = std::collections::BTreeMap::new();
    env.insert("DEMETEO_ARGV_TEST".to_string(), "present".to_string());

    let out = adapter
        .run_program(
            "local",
            ProgramRequest {
                executable: "pwsh".to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-File".to_string(),
                    script.to_string_lossy().into_owned(),
                    "value with spaces".to_string(),
                ],
                cwd: Some(cwd.to_string_lossy().into_owned()),
                env,
                timeout: Some(Duration::from_secs(5)),
            },
        )
        .await
        .expect("structured argv request succeeds");

    let _ = std::fs::remove_dir_all(&cwd);
    assert!(out.starts_with("value with spaces|present|"), "got: {out}");
    assert!(out.contains(cwd.to_string_lossy().as_ref()), "got: {out}");
}

#[cfg(windows)]
#[tokio::test]
async fn a_windows_program_failure_includes_stderr_and_exit_code() {
    let err = LocalSubprocessAdapter::new()
        .run_program(
            "local",
            ProgramRequest {
                executable: "pwsh".to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                    "[Console]::Error.Write('program boom'); exit 7".to_string(),
                ],
                ..ProgramRequest::default()
            },
        )
        .await
        .expect_err("exit 7 is a failure");
    assert!(err.contains("program boom"), "got: {err}");
    assert!(err.contains('7'), "got: {err}");
}

#[cfg(windows)]
#[tokio::test]
async fn a_windows_program_timeout_returns_promptly() {
    let started = Instant::now();
    let err = LocalSubprocessAdapter::new()
        .run_program(
            "local",
            ProgramRequest {
                executable: "pwsh".to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                    "Start-Sleep -Seconds 30".to_string(),
                ],
                timeout: Some(Duration::from_millis(300)),
                ..ProgramRequest::default()
            },
        )
        .await
        .expect_err("the ceiling is exceeded");
    assert!(err.starts_with(TIMEOUT_ERROR_PREFIX), "got: {err}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(windows)]
#[tokio::test]
async fn cancelling_a_windows_program_request_drops_its_process() {
    let adapter = LocalSubprocessAdapter::new();
    let started = Instant::now();
    let run = adapter.run_program(
        "local",
        ProgramRequest {
            executable: "pwsh".to_string(),
            args: vec![
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-Command".to_string(),
                "Start-Sleep -Seconds 30".to_string(),
            ],
            ..ProgramRequest::default()
        },
    );
    let cancelled = tokio::time::timeout(Duration::from_millis(300), run).await;

    assert!(
        cancelled.is_err(),
        "the long-running program unexpectedly finished"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[cfg(unix)]
#[tokio::test]
async fn a_nonzero_exit_is_err_with_the_output_attached() {
    let adapter = LocalSubprocessAdapter::new();
    let err = adapter
        .run_command_with("local", "echo boom >&2; exit 3", harness_opts(None))
        .await
        .expect_err("exit 3 is a failure");
    assert!(
        err.contains("boom"),
        "stderr must survive into the error: {err}"
    );
    assert!(err.contains('3'), "the exit code names itself: {err}");
    // A verdict, not a timeout — the command *ran*.
    assert!(!err.starts_with(TIMEOUT_ERROR_PREFIX));
}

#[cfg(unix)]
#[tokio::test]
async fn the_timeout_returns_promptly_rather_than_waiting_out_the_command() {
    let adapter = LocalSubprocessAdapter::new();
    let started = Instant::now();
    let err = adapter
        .run_command_with(
            "local",
            "sleep 30",
            harness_opts(Some(Duration::from_millis(300))),
        )
        .await
        .expect_err("the ceiling is exceeded");

    assert!(
        err.starts_with(TIMEOUT_ERROR_PREFIX),
        "a timeout must be distinguishable from a verdict: {err}"
    );
    // The old implementation returned only after `sleep 30` completed.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "returned after {:?}, so the deadline is not being enforced",
        started.elapsed()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn the_timeout_kills_the_whole_process_tree_not_just_the_shell() {
    // The heart of it. Every command runs as `bash -c <body>`, so killing the
    // direct child reaps the *shell* and orphans whatever it spawned. Here the
    // grandchild is the long `sleep`; it must not outlive the deadline.
    let adapter = LocalSubprocessAdapter::new();
    let dir = std::env::temp_dir().join(format!("demeteo-killtest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let pidfile = dir.join("grandchild.pid");

    let script = format!("sleep 60 & echo $! > {}; wait", pidfile.display());
    let err = adapter
        .run_command_with(
            "local",
            &script,
            harness_opts(Some(Duration::from_millis(500))),
        )
        .await
        .expect_err("the ceiling is exceeded");
    assert!(err.starts_with(TIMEOUT_ERROR_PREFIX));

    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the script recorded its grandchild pid")
        .trim()
        .parse()
        .expect("pid parses");

    // Give the signal a moment to land.
    for _ in 0..50 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let still_running = alive(pid);
    if still_running {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !still_running,
        "the `sleep 60` grandchild outlived the timeout — the shell was killed but not its tree"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn abandoning_the_future_kills_the_tree_too() {
    // Cancellation goes through the same door: the `command` node races the
    // run against its cancel watch, and dropping the losing future is what has
    // to stop the work.
    let adapter = LocalSubprocessAdapter::new();
    let dir = std::env::temp_dir().join(format!("demeteo-canceltest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let pidfile = dir.join("grandchild.pid");
    let script = format!("sleep 60 & echo $! > {}; wait", pidfile.display());

    {
        let run = adapter.run_command_with("local", &script, harness_opts(None));
        // Long enough for the script to have written the pidfile.
        let _ = tokio::time::timeout(Duration::from_millis(600), run).await;
        // `run` is dropped here.
    }

    let pid: u32 = std::fs::read_to_string(&pidfile)
        .expect("the script recorded its grandchild pid")
        .trim()
        .parse()
        .expect("pid parses");

    for _ in 0..50 {
        if !alive(pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let still_running = alive(pid);
    if still_running {
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        !still_running,
        "dropping the run future left the command running"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn output_larger_than_a_pipe_buffer_does_not_deadlock() {
    // The pipes are drained concurrently with the wait. Reading only after the
    // child exits would wedge here the moment the 64K buffer filled.
    let adapter = LocalSubprocessAdapter::new();
    let out = adapter
        .run_command_with(
            "local",
            "for i in $(seq 1 20000); do echo 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'; done",
            harness_opts(Some(Duration::from_secs(60))),
        )
        .await
        .expect("a chatty command still completes");
    assert!(out.len() > 100_000, "got {} bytes", out.len());
}

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

// ── what the spawn is confined to, and what it refuses to start ─────────────
//
// The job itself is a syscall's worth of `cfg(windows)` and only a Windows
// machine can watch it reap anything. What it *decides* — which limits, and
// which failure is a verdict — is reachable from the Linux host, which is the
// only place anybody sees it before CI.

#[test]
fn the_job_reaps_its_tree_but_lets_a_process_that_asks_break_away() {
    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const BREAKAWAY_OK: u32 = 0x800;
    const SILENT_BREAKAWAY_OK: u32 = 0x1000;

    assert_eq!(JOB_LIMIT_FLAGS & KILL_ON_JOB_CLOSE, KILL_ON_JOB_CLOSE);
    assert_eq!(JOB_LIMIT_FLAGS & BREAKAWAY_OK, BREAKAWAY_OK);
    assert_eq!(
        JOB_LIMIT_FLAGS & SILENT_BREAKAWAY_OK,
        0,
        "silent breakaway would let every agent child leave the tree unasked — see the constant"
    );
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

#[tokio::test]
async fn a_program_request_no_spawn_can_carry_never_reaches_the_retry_loop() {
    // The same classification, through the adapter that has to produce it. A
    // NUL is what makes `std` refuse on this host; on Windows it is a prompt
    // heading for a `.cmd` shim. Both arrive as `InvalidInput` from `spawn`,
    // before the program is so much as looked for.
    let err = LocalSubprocessAdapter::new()
        .run_program(
            "local",
            ProgramRequest {
                executable: "demeteo-never-runs".to_string(),
                args: vec!["a\0b".to_string()],
                ..ProgramRequest::default()
            },
        )
        .await
        .expect_err("an argument that cannot be passed cannot spawn");

    assert_eq!(classify_exec_failure(&err), HarnessExecFailure::Transport);
    assert!(err.contains("demeteo-never-runs"), "got: {err}");
}

#[cfg(windows)]
#[tokio::test]
async fn every_program_spawn_is_hardened_even_against_its_own_caller() {
    let mut env = std::collections::BTreeMap::new();
    env.insert("MSYSTEM".to_string(), "MINGW64".to_string());

    let out = LocalSubprocessAdapter::new()
        .run_program(
            "local",
            ProgramRequest {
                executable: "pwsh".to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-Command".to_string(),
                    "[Console]::Write(\"$env:NoDefaultCurrentDirectoryInExePath|$env:MSYSTEM\")"
                        .to_string(),
                ],
                env,
                timeout: Some(Duration::from_secs(30)),
                ..ProgramRequest::default()
            },
        )
        .await
        .expect("the probe runs");

    assert_eq!(
        out, "1|",
        "a child of a Git Bash ancestor must not carry MSYSTEM into git, and must not search \
         its working directory for the program it was asked for"
    );
}

#[cfg(windows)]
#[tokio::test]
async fn a_grandchild_is_reaped_when_the_deadline_takes_the_tree() {
    // Why the job exists. Windows has no process group to signal, so killing
    // the direct child leaves its own children running — an `npm test`
    // abandoned at its ceiling would leave the compiler it started writing
    // into a worktree that is about to be deleted.
    let dir = std::env::temp_dir().join(format!("demeteo-jobtest-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    let started = dir.join("grandchild-started");
    let survived = dir.join("grandchild-survived");
    let script = dir.join("spawn-grandchild.ps1");
    std::fs::write(
        &script,
        format!(
            "$g = \"New-Item -ItemType File -Path '{started}' | Out-Null; \
             Start-Sleep -Seconds 6; \
             New-Item -ItemType File -Path '{survived}' | Out-Null\"\n\
             Start-Process -FilePath 'pwsh' -NoNewWindow \
             -ArgumentList '-NoProfile','-NonInteractive','-Command',$g\n\
             Start-Sleep -Seconds 120\n",
            started = started.display(),
            survived = survived.display(),
        ),
    )
    .expect("script");

    let err = LocalSubprocessAdapter::new()
        .run_program(
            "local",
            ProgramRequest {
                executable: "pwsh".to_string(),
                args: vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-File".to_string(),
                    script.to_string_lossy().into_owned(),
                ],
                timeout: Some(Duration::from_secs(5)),
                ..ProgramRequest::default()
            },
        )
        .await
        .expect_err("the ceiling is exceeded");
    assert!(err.starts_with(TIMEOUT_ERROR_PREFIX), "got: {err}");

    tokio::time::sleep(Duration::from_secs(12)).await;
    let launched = started.exists();
    let outlived = survived.exists();
    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        launched,
        "the grandchild never started, so a clean run here would have proved nothing"
    );
    assert!(
        !outlived,
        "the grandchild outlived the deadline — the child was killed but not its tree"
    );
}

#[tokio::test]
async fn create_dir_all_creates_nested_directories_without_a_shell() {
    let adapter = LocalSubprocessAdapter::new();
    let root = std::env::temp_dir().join(format!(
        "demeteo-create-dir-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after Unix epoch")
            .as_nanos()
    ));
    let target = root.join("nested").join("directory");

    adapter
        .create_dir_all("local", &target.to_string_lossy())
        .await
        .expect("native recursive create succeeds");

    assert!(target.is_dir());
    let _ = std::fs::remove_dir_all(root);
}
