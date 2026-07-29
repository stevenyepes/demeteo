//! Running a one-shot command over an SSH channel: assembling the shell
//! invocation, opening the channel, draining both streams, and enforcing the
//! exit-code invariant. The `ExecutionPort` impl in `client.rs` keeps only the
//! async adaptation (`spawn_blocking` + the `ShellOptions::timeout` wait); the
//! shell assembly is pure and lives here so it is unit-testable without a live
//! socket.

use super::retry::SshFailure;
use super::session::SessionPool;
use super::transport::{drain_stream, transport_err, DrainBudget, TRANSPORT_WALL_CAP};
use crate::paths;
use crate::ports::execution::ShellOptions;
use crate::shared::shell;
use ssh2::Session;

/// Assemble the exact shell invocation sent over the channel. Pure — no
/// session, no I/O — so the login/non-login and interactive wrapper choices
/// are unit-testable, and so this transport's assembly can be diffed against
/// the local adapter's when parity is in question.
pub(super) fn build_invocation(cmd: &str, opts: &ShellOptions) -> String {
    // Assemble the shell invocation identically to the local
    // adapter: exports run *inside* the body (after a login shell
    // sources its profile) so the caller's env wins; `cd` is baked
    // into the body so a failed `cd` aborts before the command runs.
    let exports = shell::export_prefix(&opts.env);
    let body = format!(
        "{}{}",
        shell::job_control_prefix(opts.interactive),
        shell::command_body(opts.cwd.as_deref(), &exports, cmd)
    );
    if opts.login_shell {
        // `-i` (interactive) sources `~/.bashrc`, where tool-managers
        // (mise/asdf/nvm) put their PATH activation behind the standard
        // non-interactive guard; a plain `-l` login shell misses them.
        // See `ShellOptions::interactive`.
        let flags = if opts.interactive {
            "-l -i -c"
        } else {
            "-l -c"
        };
        format!("bash {} {}", flags, paths::shell_escape_posix(&body))
    } else {
        format!("sh -c {}", paths::shell_escape_posix(&body))
    }
}

/// The blocking half of [`ExecutionPort::run_command_with`]: get (or open) the
/// pooled session, assemble the invocation, and run it. Everything here is
/// synchronous `ssh2` work — the caller owns the `spawn_blocking` boundary,
/// the timeout wait, and the retry loop.
///
/// The error carries a [`SshFailure`] rather than a bare `String` because a
/// retry decision cannot be made from the message: the same "the connection
/// broke" text means "nothing ran, re-run me" before `exec` and "the remote is
/// running this right now" after it. Only this function knows which side of
/// that line it failed on, so only it can say. See `super::retry`.
///
/// [`ExecutionPort::run_command_with`]: crate::ports::execution::ExecutionPort::run_command_with
pub(super) fn run_blocking(
    pool: &SessionPool,
    machine_id: &str,
    cmd: &str,
    opts: &ShellOptions,
) -> Result<String, SshFailure> {
    // A failure to establish/reuse the session is a transport failure,
    // not a command failure — tag it so callers (e.g. the verifier)
    // don't misclassify an unreachable machine as a red build. It is also the
    // one failure that is unambiguously safe to retry: no session means the
    // command was never handed to a shell.
    let sftp_sess = pool.get(machine_id).map_err(|e| {
        let msg = if e.starts_with(crate::ports::execution::TRANSPORT_ERROR_PREFIX) {
            e
        } else {
            transport_err(e)
        };
        SshFailure::never_reached(msg)
    })?;

    let full_cmd = build_invocation(cmd, opts);

    exec_over_channel(&sftp_sess.session, &full_cmd)
}

/// Open a fresh channel on `session`, exec `full_cmd`, drain stdout AND
/// stderr, and enforce the exit-code invariant (D3): a non-zero exit is an
/// `Err` carrying the captured stderr (never `Ok("")`). `full_cmd` is the
/// already-assembled shell invocation (login/non-login wrapper + cwd + env);
/// this helper is transport plumbing only and makes no assumptions about it.
///
/// This is the single drain-and-check path shared by `run_command_with`, so
/// the exit-status handling that root-caused "UI says HEALTHY but the dir
/// doesn't exist" lives in exactly one place.
///
/// It is also where the retry boundary is drawn, and the boundary is exactly
/// `channel.exec`. Everything before it — opening the channel — leaves the
/// remote shell with nothing, so a failure there is
/// [`SshFailure::never_reached`] and safe to re-run whatever the command does.
/// Everything from `exec` onward is [`SshFailure::may_have_run`]: libssh2's
/// process-startup request may have been delivered without its reply coming
/// back, and once the command is running the remote process outlives the
/// channel — so a retry would run a second copy alongside the first.
pub(super) fn exec_over_channel(session: &Session, full_cmd: &str) -> Result<String, SshFailure> {
    let mut channel = session.channel_session().map_err(|e| {
        SshFailure::never_reached(transport_err(format!("Failed to open SSH channel: {}", e)))
    })?;
    channel.exec(full_cmd).map_err(|e| {
        SshFailure::may_have_run(transport_err(format!("Failed to execute command: {}", e)))
    })?;

    // Timeout-tolerant drain: a long silent command (e.g. `cargo test`
    // compiling) must not be aborted by the session's 10s blocking-call
    // timeout. See `drain_stream` / `TRANSPORT_WALL_CAP`.
    //
    // One budget for both streams: stdout and stderr share it, so a command
    // cannot spend the full cap draining each in turn.
    let budget = DrainBudget::starting_now(TRANSPORT_WALL_CAP);
    let mut stdout_bytes = Vec::new();
    drain_stream(
        &mut channel,
        session,
        &mut stdout_bytes,
        budget,
        "command stdout",
    )
    .map_err(SshFailure::may_have_run)?;
    let stdout = String::from_utf8_lossy(&stdout_bytes).into_owned();

    // ssh2 keeps stderr on a separate stream. Drain it so the remote
    // shell's error message is included in the Err variant.
    let mut stderr_bytes = Vec::new();
    {
        let mut err_stream = channel.stderr();
        let _ = drain_stream(
            &mut err_stream,
            session,
            &mut stderr_bytes,
            budget,
            "command stderr",
        );
    }
    let stderr = String::from_utf8_lossy(&stderr_bytes).into_owned();

    channel.wait_close().map_err(|e| {
        SshFailure::may_have_run(transport_err(format!(
            "Failed to wait for channel close: {}",
            e
        )))
    })?;
    let exit_code = channel.exit_status().map_err(|e| {
        SshFailure::may_have_run(transport_err(format!(
            "Failed to read command exit status: {}",
            e
        )))
    })?;

    if exit_code != 0 {
        let detail = if stderr.trim().is_empty() {
            format!("exit code: {}", exit_code)
        } else {
            stderr.trim().to_string()
        };
        // The command reached a verdict. Never retried — that is the whole
        // point of `Answered`.
        return Err(SshFailure::answered(format!(
            "Command failed ({}): {}",
            detail, full_cmd
        )));
    }

    Ok(stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `run_command` (no override) delegates with `ShellOptions::default()`,
    /// which must stay a plain non-login `sh -c` — the historical behaviour of
    /// the bare `channel.exec` this path replaced.
    #[test]
    fn default_options_use_a_non_login_sh() {
        let inv = build_invocation("echo hi", &ShellOptions::default());
        assert!(
            inv.starts_with("sh -c "),
            "expected a non-login `sh -c` wrapper, got: {inv}",
        );
        assert!(!inv.contains("bash"), "no bash for the default: {inv}");
        assert!(inv.contains("echo hi"), "command must survive: {inv}");
    }

    /// The distinction that fixed a real bug: agents were reported "not found"
    /// on machines where they were installed, because `~/.bashrc` (where
    /// mise/asdf/nvm put their PATH activation, behind the standard
    /// non-interactive guard) is only sourced by an *interactive* shell. A
    /// login-but-non-interactive shell must stay `-l -c`, since an interactive
    /// one also echoes any banner into stdout that callers parse.
    #[test]
    fn login_shell_adds_interactive_flag_only_when_asked() {
        let interactive = build_invocation(
            "command -v opencode",
            &ShellOptions {
                login_shell: true,
                interactive: true,
                ..Default::default()
            },
        );
        assert!(
            interactive.starts_with("bash -l -i -c "),
            "interactive login must source ~/.bashrc, got: {interactive}",
        );

        let quiet = build_invocation(
            "command -v opencode",
            &ShellOptions {
                login_shell: true,
                interactive: false,
                ..Default::default()
            },
        );
        assert!(
            quiet.starts_with("bash -l -c "),
            "non-interactive login must not get `-i`, got: {quiet}",
        );
    }

    /// The `cd` belongs *inside* the shell body, ahead of the command and
    /// joined by `&&`, so a failed `cd` aborts instead of silently running the
    /// command in the login directory.
    #[test]
    fn cwd_is_baked_into_the_body_ahead_of_the_command() {
        let inv = build_invocation(
            "make build",
            &ShellOptions {
                cwd: Some("/srv/worktrees/wt-1".to_string()),
                ..Default::default()
            },
        );
        let cd_at = inv
            .find("cd /srv/worktrees/wt-1")
            .expect("cd must be present");
        let cmd_at = inv.find("make build").expect("command must be present");
        assert!(cd_at < cmd_at, "cd must precede the command: {inv}");
        assert!(
            inv[cd_at..cmd_at].contains("&&"),
            "cd must gate the command with `&&`: {inv}",
        );
    }

    /// Exports run *after* the login shell has sourced its profile — i.e.
    /// inside the quoted body handed to `-c`, not on the wrapper's own command
    /// line — so the caller's values win over the profile's.
    #[test]
    fn env_exports_live_inside_the_wrapped_body() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("DEMETEO_TOKEN".to_string(), "abc".to_string());
        let inv = build_invocation(
            "printenv DEMETEO_TOKEN",
            &ShellOptions {
                login_shell: true,
                env,
                ..Default::default()
            },
        );
        let prefix = "bash -l -c ";
        assert!(inv.starts_with(prefix), "unexpected wrapper: {inv}");
        let export_at = inv.find("export DEMETEO_TOKEN=").expect("export missing");
        assert!(
            export_at > prefix.len(),
            "exports must sit inside the wrapped body, not before it: {inv}",
        );
        let cmd_at = inv.find("printenv").expect("command must be present");
        assert!(
            export_at < cmd_at,
            "exports must precede the command: {inv}"
        );
    }

    /// A value carrying the one character single-quoting cannot contain must
    /// still arrive intact: the body is quoted once for `-c`, and the value
    /// again for `export`, so the escaping has to survive both layers.
    #[test]
    fn a_value_needing_quoting_survives_escaping() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("MSG".to_string(), "it's a trap".to_string());
        let inv = build_invocation(
            "echo \"$MSG\"",
            &ShellOptions {
                env,
                ..Default::default()
            },
        );
        assert!(inv.starts_with("sh -c "), "unexpected wrapper: {inv}");
        assert!(
            inv.contains("MSG=") && inv.contains("trap"),
            "the value must still be there: {inv}",
        );
        assert!(
            !inv.contains("it's a trap"),
            "the embedded quote must be escaped, not passed through raw: {inv}",
        );
    }
}
