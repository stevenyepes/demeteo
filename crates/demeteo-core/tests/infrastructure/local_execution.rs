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
use crate::ports::execution::TIMEOUT_ERROR_PREFIX;
use std::time::{Duration, Instant};

/// `interactive` is what makes the child a session leader (`setsid`), which is
/// what makes the process-group kill safe — and it is what every `command`
/// node uses, via `harness_shell_options`.
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

#[tokio::test]
async fn a_command_that_finishes_returns_its_stdout() {
    let adapter = LocalSubprocessAdapter::new();
    let out = adapter
        .run_command_with("local", "echo hello", harness_opts(None))
        .await
        .expect("echo succeeds");
    assert_eq!(out.trim(), "hello");
}

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
