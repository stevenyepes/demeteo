//! Interactive (PTY-backed) agent sessions over SSH: assembling the PTY
//! command line, opening a dedicated session + channel for it, and the
//! [`InteractiveHandle`] that reads/writes that channel. The `ExecutionPort`
//! impl in `client.rs` keeps only the trait method that forwards here; the
//! command assembly is pure and lives here so it is unit-testable without a
//! live socket.

use super::session::{machine_secret, SessionPool};
use crate::paths;
use crate::ports::execution::InteractiveHandle;
use ssh2::{Channel, Session};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::Mutex;

pub(super) struct RemoteChannelHandle {
    channel: Mutex<Channel>,
    session: Session,
}

impl InteractiveHandle for RemoteChannelHandle {
    fn write_line(&self, line: &str) -> std::io::Result<usize> {
        let mut channel = self.channel.lock().unwrap();
        channel.write_all(line.as_bytes())?;
        channel.write_all(b"\n")?;
        channel.flush()?;
        Ok(line.len() + 1)
    }

    fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut channel = self.channel.lock().unwrap();
        match channel.read(buf) {
            Ok(n) => Ok(n),
            // A timeout here is NOT end-of-stream: the session carries a 10s
            // blocking-call timeout (see `ssh_util::connect`), and with
            // keepalive configured libssh2 aborts a blocking read the moment
            // a keepalive comes due (~30s after handshake) even while data
            // is flowing. Send the keepalive libssh2 is waiting on and
            // report `WouldBlock` so the caller retries instead of treating
            // a healthy mid-turn stream as ended. Covered live by the
            // ignored `remote_pty_stream_survives_keepalive_and_silence_live`
            // test.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                let _ = self.session.keepalive_send();
                Err(std::io::Error::new(std::io::ErrorKind::WouldBlock, e))
            }
            Err(e) => Err(e),
        }
    }

    fn kill(&self) -> Result<(), String> {
        let mut channel = self.channel.lock().unwrap();
        channel.close().map_err(|e| e.to_string())
    }

    fn try_wait(&self) -> Result<Option<i32>, String> {
        let channel = self.channel.lock().unwrap();
        // `exit_status()` returns Ok(0) while the remote process is still
        // running (libssh2 only learns the real status once the channel
        // reaches EOF). Answer "still running" until then so callers never
        // mistake a live agent for a clean exit.
        if !channel.eof() {
            return Ok(None);
        }
        match channel.exit_status() {
            Ok(code) => Ok(Some(code)),
            Err(e) => Err(e.to_string()),
        }
    }
}

/// Assemble the PTY command line for an interactive agent session. Pure — no
/// session, no I/O — so the login-shell wrapper and env/arg escaping are
/// unit-testable.
fn build_pty_command(
    binary: &str,
    args: &[String],
    cwd: &str,
    env: &HashMap<String, String>,
    use_login_shell: bool,
) -> String {
    // Env composition (HOME / USER / LOGNAME resolution against
    // the remote machine's identity) is the caller's
    // responsibility — `agent_base_env` in `ports/agent_runtime.rs`
    // is the single owner and consults `ExecutionPort::resolve_home`
    // + `ExecutionPort::resolve_user`. This adapter is now a
    // "pure" executor: it forwards whatever env the caller built,
    // no business logic, no identity assumptions. The previous
    // override here (HOME/USER/LOGNAME) was a defense-in-depth
    // band-aid for a class of bugs the port-side refactor
    // eliminates by construction.
    let mut env_str = String::new();
    for (k, v) in env {
        let escaped = v.replace('\'', "'\\''");
        env_str.push_str(&format!("export {}='{}'; ", k, escaped));
    }
    let args_str = args
        .iter()
        .map(|a| paths::shell_escape_posix(a))
        .collect::<Vec<_>>()
        .join(" ");

    if use_login_shell {
        let inner = format!(
            "{} command cd {} && {{ command -v mise >/dev/null 2>&1 && mise trust --yes . || :; }} 2>/dev/null && exec {} {}",
            env_str,
            paths::shell_escape_posix(cwd),
            paths::shell_escape_posix(binary),
            args_str
        );
        // Interactive (`-i`) so `~/.bashrc` is sourced — that's where
        // mise/asdf/nvm activate the toolchain that puts the agent binary
        // on PATH. A non-interactive `bash -l` login shell hits the
        // standard `.bashrc` non-interactive guard and never activates
        // them, so `exec opencode` would fail with "command not found"
        // even though the binary is installed. A PTY is always requested
        // by the caller, so `-i` has a controlling terminal and stays quiet.
        // This mirrors the interactive availability probe (see
        // `ShellOptions::interactive`) so "available" and "runnable" agree.
        format!("bash -l -i -c {}", paths::shell_escape_posix(&inner))
    } else {
        format!(
            "cd {} && {} {} {}",
            paths::shell_escape_posix(cwd),
            env_str,
            paths::shell_escape_posix(binary),
            args_str
        )
    }
}

/// The body of [`ExecutionPort::spawn_interactive`]: resolve the machine,
/// open a **fresh** session (this path deliberately does not use a pooled
/// one — an interactive agent holds its channel for the whole turn and must
/// not share a session with the one-shot/SFTP traffic), request a PTY, and
/// exec the assembled command line on it.
///
/// Synchronous by design: the port method itself is not `async`, so there is
/// no `spawn_blocking` boundary here. `pool` is only used to resolve the
/// `Machine` record.
///
/// [`ExecutionPort::spawn_interactive`]: crate::ports::execution::ExecutionPort::spawn_interactive
pub(super) fn spawn(
    pool: &SessionPool,
    machine_id: &str,
    binary: &str,
    args: &[String],
    cwd: &str,
    env: &HashMap<String, String>,
) -> Result<Box<dyn InteractiveHandle>, String> {
    let machine = crate::infrastructure::worktree::machine_resolver::resolve_machine(
        pool.machines(),
        machine_id,
    )?;

    let secret = machine_secret(&machine);

    // Keepalive (kernel + app-level) is configured centrally in
    // `ssh_util::connect`, so no per-session `set_keepalive` here.
    let (sess, _tcp) = crate::ssh_util::connect(&machine, secret)?;

    let mut channel = sess
        .channel_session()
        .map_err(|e| format!("Failed to open SSH channel: {}", e))?;

    channel
        .request_pty("xterm-256color", None, None)
        .map_err(|e| format!("Failed to request PTY on SSH channel: {}", e))?;

    let use_login_shell = machine.use_login_shell.unwrap_or(false);

    let cmd = build_pty_command(binary, args, cwd, env, use_login_shell);

    eprintln!("[SshClientAdapter] spawn_interactive cmd: {}", cmd);
    channel
        .exec(&cmd)
        .map_err(|e| format!("Failed to exec agent over SSH: {}", e))?;

    Ok(Box::new(RemoteChannelHandle {
        channel: Mutex::new(channel),
        session: sess,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The non-login branch is a bare `cd <cwd> && <exports> <binary> <args>`:
    /// no shell wrapper, so the `cd` gates the command with `&&` directly.
    #[test]
    fn non_login_branch_cds_then_runs_the_binary() {
        let cmd = build_pty_command(
            "opencode",
            &["run".to_string(), "--json".to_string()],
            "/srv/worktrees/wt-1",
            &HashMap::new(),
            false,
        );
        assert!(
            cmd.starts_with("cd /srv/worktrees/wt-1 && "),
            "non-login must cd first, got: {cmd}",
        );
        assert!(!cmd.contains("bash -l"), "no login wrapper: {cmd}");
        let bin_at = cmd.find("opencode").expect("binary must be present");
        let args_at = cmd.find("--json").expect("args must be present");
        assert!(bin_at < args_at, "binary must precede its args: {cmd}");
        assert!(cmd.contains("run --json"), "args must survive: {cmd}");
    }

    /// The login branch wraps everything in `bash -l -i -c` (see the comment on
    /// `build_pty_command`: `-i` is what sources `~/.bashrc` and puts the
    /// mise/asdf/nvm-managed agent binary on PATH), and carries the `mise trust`
    /// probe plus the `exec` that replaces the shell with the agent.
    #[test]
    fn login_branch_wraps_in_interactive_login_bash_with_mise_probe() {
        let cmd = build_pty_command(
            "opencode",
            &["run".to_string()],
            "/srv/worktrees/wt-1",
            &HashMap::new(),
            true,
        );
        assert!(
            cmd.starts_with("bash -l -i -c "),
            "login branch must use an interactive login shell, got: {cmd}",
        );
        assert!(
            cmd.contains("mise trust --yes"),
            "the mise trust probe must survive: {cmd}",
        );
        assert!(cmd.contains("exec"), "the agent must be exec'd: {cmd}");
        assert!(
            cmd.contains("command cd"),
            "login branch uses `command cd`: {cmd}",
        );
    }

    /// A value carrying the one character single-quoting cannot contain must be
    /// escaped as `'\''` rather than passed through raw, or the export would
    /// terminate its own quoting and the rest of the line would be reinterpreted.
    ///
    /// Note: env vars are emitted by iterating a `HashMap`, whose order is
    /// nondeterministic — so this (and the test below) assert on the presence of
    /// each `export K='V';` fragment, never on one exact whole string.
    #[test]
    fn a_single_quote_in_an_env_value_is_escaped() {
        let cmd = build_pty_command(
            "agent",
            &[],
            "/tmp/wt",
            &env_of(&[("MSG", "it's a trap")]),
            false,
        );
        assert!(
            cmd.contains(r#"export MSG='it'\''s a trap';"#),
            "the embedded quote must be escaped as '\\'': {cmd}",
        );
        assert!(
            !cmd.contains("'it's a trap'"),
            "the raw quote must not survive: {cmd}",
        );
    }

    /// Every caller-supplied var reaches the command line as its own
    /// `export K='V';` fragment. Asserted per-fragment because `HashMap`
    /// iteration order is nondeterministic and deliberately left that way.
    #[test]
    fn every_env_var_is_exported_regardless_of_map_order() {
        let cmd = build_pty_command(
            "agent",
            &[],
            "/tmp/wt",
            &env_of(&[("HOME", "/home/dev"), ("USER", "dev"), ("LOGNAME", "dev")]),
            false,
        );
        for fragment in [
            "export HOME='/home/dev';",
            "export USER='dev';",
            "export LOGNAME='dev';",
        ] {
            assert!(cmd.contains(fragment), "missing {fragment} in: {cmd}");
        }
    }

    /// Args go through `paths::shell_escape_posix`, so one carrying a space or
    /// a quote arrives as a single argv entry instead of splitting.
    #[test]
    fn args_are_shell_escaped() {
        let arg = "a prompt with spaces";
        let cmd = build_pty_command(
            "agent",
            &[arg.to_string()],
            "/tmp/wt",
            &HashMap::new(),
            false,
        );
        assert!(
            cmd.contains(&paths::shell_escape_posix(arg)),
            "arg must be escaped, got: {cmd}",
        );
        assert!(
            cmd.ends_with("'a prompt with spaces'"),
            "an unescaped arg would split into four words: {cmd}",
        );
    }

    /// A cwd containing a space must survive both branches — unescaped it would
    /// make `cd` see two operands and the whole session would start in the
    /// wrong directory (or fail).
    #[test]
    fn a_cwd_with_a_space_survives_escaping() {
        let cwd = "/srv/work trees/wt-1";
        let escaped = paths::shell_escape_posix(cwd);

        let plain = build_pty_command("agent", &[], cwd, &HashMap::new(), false);
        assert!(
            plain.starts_with(&format!("cd {} && ", escaped)),
            "non-login cwd must be escaped, got: {plain}",
        );

        // The login branch escapes the cwd once for the inner body and then
        // escapes the whole body again for `bash -c`, so the quotes around the
        // cwd come back doubled.
        let login = build_pty_command("agent", &[], cwd, &HashMap::new(), true);
        let twice_escaped = escaped.replace('\'', "'\\''");
        assert!(
            login.contains(&format!("command cd {} &&", twice_escaped)),
            "login cwd must survive both escaping layers, got: {login}",
        );
    }
}
