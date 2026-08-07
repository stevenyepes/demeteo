//! Starting one child and reading it back.
//!
//! The three spawn paths every `ExecutionPort` method funnels into, plus the
//! two helpers that shape a child and its result. None of them knows what a
//! machine id is — a request goes in and its output comes back — which is the
//! boundary that keeps their cancellation and deadline behaviour readable
//! together rather than interleaved with the port's dispatch.

use std::process::{Command, Stdio};

use crate::ports::execution::{ProgramRequest, ShellOptions};
use crate::shared::proc::{harden_child_spawn, sanitize_child_env};

use super::invocation::{program_path, shell_invocation, unspawnable_arguments};
use super::process_guard::{KillGroupOnDrop, ProcessGuard};

/// Apply the non-argument half of `opts` to a spawned child.
pub(super) fn configure_child(command: &mut Command, opts: &ShellOptions) {
    if let Some(cwd) = &opts.cwd {
        command.current_dir(cwd);
    }
    // An interactive login shell (`bash -l -i -c`, used by the availability /
    // model probes so mise/asdf/nvm tools resolve) tries to grab the
    // controlling terminal for job control. When demeteo runs under a terminal
    // (e.g. `tauri dev`), that suspends the whole process group. Detach the
    // child into its own session so it has no controlling TTY. Harmless for the
    // non-interactive paths. See `detach_from_controlling_tty`.
    if opts.interactive {
        crate::shared::proc::detach_from_controlling_tty(command);
    }
    sanitize_child_env(command);
    harden_child_spawn(command);
}

/// Assemble the D3 result shape from a finished child: stdout on success,
/// `Err(stdout + stderr)` on a non-zero exit — never `Ok("")`.
pub(super) fn command_result(
    status_code: Option<i32>,
    ok: bool,
    stdout: &[u8],
    stderr: &[u8],
) -> Result<String, String> {
    let mut result = String::from_utf8_lossy(stdout).to_string();
    if !ok {
        let stderr = String::from_utf8_lossy(stderr);
        if !result.is_empty() {
            result.push('\n');
        }
        result.push_str(&stderr);
        return Err(format!(
            "Command failed (exit code: {:?}): {}",
            status_code, result
        ));
    }
    Ok(result)
}

/// Execute an argv request directly so owned operations never depend on shell quoting.
pub(super) async fn local_run_program(request: ProgramRequest) -> Result<String, String> {
    use tokio::io::AsyncReadExt;
    let mut command = tokio::process::Command::new(program_path(&request.executable));
    command.args(&request.args);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.envs(&request.env);
    sanitize_child_env(command.as_std_mut());
    harden_child_spawn(command.as_std_mut());
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let guard = ProcessGuard::armed();
    let mut child = command.spawn().map_err(|e| {
        unspawnable_arguments(&request.executable, &e)
            .unwrap_or_else(|| format!("Failed to execute '{}': {}", request.executable, e))
    })?;
    guard.adopt(&child);
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe was not available".to_string())?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe was not available".to_string())?;
    let run = async {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let (_, _, status) = tokio::join!(
            stdout.read_to_end(&mut out),
            stderr.read_to_end(&mut err),
            child.wait()
        );
        (out, err, status)
    };
    let (out, err, status) = match request.timeout {
        Some(limit) => tokio::time::timeout(limit, run).await.map_err(|_| {
            format!(
                "{}program exceeded its {}s ceiling",
                crate::ports::execution::TIMEOUT_ERROR_PREFIX,
                limit.as_secs()
            )
        })?,
        None => run.await,
    };
    guard.disarm();
    let status = status.map_err(|e| format!("Failed to await program: {}", e))?;
    command_result(status.code(), status.success(), &out, &err)
}

/// Run `cmd` locally honouring `opts`, **owning the deadline** so an expiry
/// actually stops the work (see [`ShellOptions::timeout`]).
///
/// Cancel-safe by construction: the group kill hangs off `Drop`, so abandoning
/// this future — a timeout, a cancelled step, an aborted task — kills the
/// command tree just as the deadline does. That is what lets the `command`
/// node treat "cancelled" as immediate.
pub(super) async fn local_run_command_async(
    cmd: &str,
    opts: &ShellOptions,
) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    let (program, args) = shell_invocation(cmd, opts)?;
    let mut command = tokio::process::Command::new(&program);
    command.args(&args);
    configure_child(command.as_std_mut(), opts);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // The floor when the group kill is disarmed (non-`setsid` children).
        .kill_on_drop(true);

    let job = ProcessGuard::armed();
    let mut child = command.spawn().map_err(|e| {
        unspawnable_arguments(&program.to_string_lossy(), &e)
            .unwrap_or_else(|| format!("Failed to execute command: {}", e))
    })?;
    job.adopt(&child);
    let mut guard = KillGroupOnDrop {
        pid: child.id(),
        own_session: opts.interactive,
    };

    let mut out_pipe = child.stdout.take().expect("stdout piped above");
    let mut err_pipe = child.stderr.take().expect("stderr piped above");
    // Drain both pipes *while* waiting. Waiting first and reading after would
    // deadlock the moment a build fills the 64K pipe buffer.
    let run = async {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let (_, _, status) = tokio::join!(
            out_pipe.read_to_end(&mut stdout),
            err_pipe.read_to_end(&mut stderr),
            child.wait(),
        );
        (stdout, stderr, status)
    };

    let (stdout, stderr, status) = match opts.timeout {
        Some(limit) => match tokio::time::timeout(limit, run).await {
            Ok(finished) => finished,
            Err(_) => {
                // `guard` drops on return and takes the process group with it.
                return Err(format!(
                    "{}command exceeded its {}s ceiling",
                    crate::ports::execution::TIMEOUT_ERROR_PREFIX,
                    limit.as_secs()
                ));
            }
        },
        None => run.await,
    };

    guard.disarm();
    job.disarm();
    let status = status.map_err(|e| format!("Failed to await command: {}", e))?;
    command_result(status.code(), status.success(), &stdout, &stderr)
}

/// Blocking structured-program helper for a few short adapter-owned setup
/// operations. User-authored scripts always go through [`local_run_program`].
///
/// The one spawn here that takes no [`ProcessGuard`]. `Command::output` never
/// yields a handle to confine, and the call it would confine cannot be
/// abandoned — there is no deadline and no cancellation point — so the job
/// would only ever fire on an unwind, where it would kill whatever `git` had
/// legitimately left running.
pub(super) fn local_run_program_blocking(request: ProgramRequest) -> Result<String, String> {
    let mut command = Command::new(program_path(&request.executable));
    command.args(&request.args);
    if let Some(cwd) = &request.cwd {
        command.current_dir(cwd);
    }
    command.envs(&request.env);
    sanitize_child_env(&mut command);
    harden_child_spawn(&mut command);
    let output = command.output().map_err(|e| {
        unspawnable_arguments(&request.executable, &e)
            .unwrap_or_else(|| format!("Failed to execute '{}': {}", request.executable, e))
    })?;
    command_result(
        output.status.code(),
        output.status.success(),
        &output.stdout,
        &output.stderr,
    )
}
