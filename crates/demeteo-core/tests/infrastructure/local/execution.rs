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
use crate::adapters::local::invocation::UNSPAWNABLE_ARGUMENTS_ERROR;
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::ports::execution::{ProgramRequest, TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};
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
    // The `param` block ends at the newline; PowerShell will not start a
    // statement on the same line as it.
    std::fs::write(
        &script,
        "param([string]$Value)\n[Console]::Write(\"$Value|$env:DEMETEO_ARGV_TEST|$PWD\")\n",
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

    // `%TEMP%` is handed out in its 8.3 short form on a stock profile
    // (`C:\Users\RUNNER~1\…`) while the kernel canonicalises a process's
    // working directory to the long one, so the child reports a different
    // spelling of the very directory it was handed. Resolved through the
    // filesystem and compared as a path, or this asserts a spelling.
    let resolved = std::fs::canonicalize(&cwd)
        .expect("the scratch directory resolves")
        .to_string_lossy()
        .into_owned();
    let expected_cwd = resolved.strip_prefix(r"\\?\").unwrap_or(&resolved);
    let _ = std::fs::remove_dir_all(&cwd);

    let mut fields = out.split('|');
    assert_eq!(fields.next(), Some("value with spaces"), "got: {out}");
    assert_eq!(fields.next(), Some("present"), "got: {out}");
    let reported = fields.next().unwrap_or_default();
    assert!(
        crate::paths::same_path(reported, expected_cwd, true),
        "the child ran in {reported}, not in {expected_cwd}"
    );
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

/// The unspawnable-argument classification through the adapter that has to
/// produce it — `invocation.rs` covers the decision itself.
#[tokio::test]
async fn a_program_request_no_spawn_can_carry_never_reaches_the_retry_loop() {
    // The same classification, through the adapter that has to produce it. A
    // NUL is what makes `std` refuse on this host; on Windows it is a prompt
    // heading for a `.cmd` shim. Both arrive as `InvalidInput` from `spawn`.
    //
    // The program is this test binary because it has to *exist*: Windows
    // resolves the executable before it encodes the arguments, so a name that
    // is not on PATH fails as `NotFound` and the argument is never reached —
    // which would test the missing-program path on one platform and the
    // unspawnable-argument path on the other.
    let executable = std::env::current_exe()
        .expect("the test binary has a path")
        .to_string_lossy()
        .into_owned();
    let err = LocalSubprocessAdapter::new()
        .run_program(
            "local",
            ProgramRequest {
                executable: executable.clone(),
                args: vec!["a\0b".to_string()],
                ..ProgramRequest::default()
            },
        )
        .await
        .expect_err("an argument that cannot be passed cannot spawn");

    assert_eq!(
        classify_exec_failure(&err),
        HarnessExecFailure::Transport,
        "got: {err}"
    );
    assert!(
        err.strip_prefix(TRANSPORT_ERROR_PREFIX)
            .is_some_and(|rest| rest.starts_with(UNSPAWNABLE_ARGUMENTS_ERROR)),
        "the NUL must be refused as an argument, not as a missing program: {err}"
    );
    assert!(err.contains(&executable), "got: {err}");
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
