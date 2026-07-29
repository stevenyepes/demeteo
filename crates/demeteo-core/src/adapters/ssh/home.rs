//! Remote `$HOME` resolution: the `printf %s "$HOME"` probe, the rules a
//! usable answer has to satisfy, and the per-machine cache in front of both.
//! Kept with [`SessionPool`] (whose `home_cache` it owns the reads of) so the
//! whole HOME concern is one file.

use super::retry::SshFailure;
use super::session::SessionPool;
use super::transport::{drain_stream, DrainBudget};
use ssh2::Session;
use std::time::Duration;

/// Wall-clock allowance for the HOME probe, deliberately far below the
/// transport's 30-minute [`TRANSPORT_WALL_CAP`].
///
/// That cap is sized for a one-shot command that may legitimately go quiet for
/// half an hour — `cargo test` compiling in silence. This probe is a single
/// `printf` on a non-login, non-interactive shell, so it sources no profile and
/// has nothing to be slow about; a second is generous and 60s is pure headroom
/// for a loaded box or a stalled network home directory. Borrowing the
/// transport cap meant a remote that connected but never answered — a hung
/// login profile, an NFS-wedged `$HOME` — kept acking keepalives, so the
/// dead-connection abort never fired and the probe rode all 30 minutes.
///
/// Below [`NO_PROGRESS_ABORT`] on purpose, which inverts the usual ordering:
/// for a probe this small, waiting two minutes to *distinguish* a dead
/// connection from a quiet one is itself the failure. Whichever it is, 60s of
/// silence is already an answer.
///
/// [`TRANSPORT_WALL_CAP`]: super::transport::TRANSPORT_WALL_CAP
/// [`NO_PROGRESS_ABORT`]: super::transport::NO_PROGRESS_ABORT
const HOME_PROBE_CAP: Duration = Duration::from_secs(60);

/// Probe `$HOME` over a fresh channel on an already-connected session.
/// `printf %s` avoids trailing newlines and respects quoting.
///
/// Graded for the retry loop the same way `command::exec_over_channel` is, and
/// for the same reason even though this command has no side effects: the probe
/// runs on [`HOME_PROBE_CAP`], so retrying a failure that happened *during* the
/// drain would spend that budget once per attempt. Only the channel open —
/// where nothing has been spent and nothing has run — is retried. See
/// `super::retry`.
fn probe_home_over_channel(session: &Session) -> Result<String, SshFailure> {
    let mut channel = session.channel_session().map_err(|e| {
        SshFailure::never_reached(format!("Failed to open SSH channel for HOME probe: {}", e))
    })?;
    channel.exec("printf %s \"$HOME\"").map_err(|e| {
        SshFailure::may_have_run(format!("Failed to exec HOME probe over SSH: {}", e))
    })?;
    let mut raw_bytes = Vec::new();
    drain_stream(
        &mut channel,
        session,
        &mut raw_bytes,
        DrainBudget::starting_now(HOME_PROBE_CAP),
        "HOME probe output",
    )
    .map_err(SshFailure::may_have_run)?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    channel.wait_close().map_err(|e| {
        SshFailure::may_have_run(format!("Failed to wait for HOME probe channel: {}", e))
    })?;
    // ssh2's `wait_close` returns `Result<(), Error>`; the exit status is
    // on a separate method that returns `Result<i32, Error>` (0 on
    // success, non-zero on remote failure). Drain it so a broken shell
    // session doesn't get cached as a valid HOME.
    let exit_code = channel.exit_status().map_err(|e| {
        SshFailure::may_have_run(format!("Failed to read HOME probe exit status: {}", e))
    })?;

    validate_home(&raw, exit_code).map_err(SshFailure::answered)
}

/// Validate the raw bytes of a `printf %s "$HOME"` probe. Split from the
/// channel round-trip so the rules are testable without a live socket.
fn validate_home(raw: &str, exit_code: i32) -> Result<String, String> {
    if exit_code != 0 {
        return Err(format!(
            "Remote HOME probe exited with status {}; the SSH session may be denying shell access",
            exit_code
        ));
    }

    let trimmed = raw.trim().to_string();
    if trimmed.is_empty() {
        return Err("Remote HOME is empty (HOME is not set on the SSH session)".to_string());
    }
    if !trimmed.starts_with('/') {
        return Err(format!(
            "Remote HOME is not an absolute path (got '{}')",
            trimmed
        ));
    }
    Ok(trimmed)
}

impl SessionPool {
    /// Resolve the remote user's HOME, serving it from `home_cache` when we've
    /// already paid for the round-trip on this machine.
    ///
    /// Blocking, like every other `ssh2` call, and on a cache miss it can cost
    /// a connect and an auth handshake on top of the probe itself — so every
    /// caller must reach it from inside `spawn_blocking`. Both do today:
    /// `ExecutionPort::resolve_home` and `control_rpc::call`.
    pub(super) fn resolve_home(&self, machine_id: &str) -> Result<String, SshFailure> {
        if let Ok(cache) = self.home_cache.lock() {
            if let Some(home) = cache.get(machine_id) {
                eprintln!(
                    "[SshClientAdapter] resolve_remote_home({}) = {} (cache hit)",
                    machine_id, home
                );
                return Ok(home.clone());
            }
        }

        let sftp_sess = self.get(machine_id).map_err(SshFailure::never_reached)?;
        let trimmed = probe_home_over_channel(&sftp_sess.session)?;

        eprintln!(
            "[SshClientAdapter] resolve_remote_home({}) = {} (fresh probe; cached)",
            machine_id, trimmed
        );
        if let Ok(mut cache) = self.home_cache.lock() {
            cache.insert(machine_id.to_string(), trimmed.clone());
        }
        Ok(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe's budget has to stay below both transport thresholds, and the
    /// reason differs for each. Below the wall cap is the whole point — a
    /// `printf` must not be allowed the half hour a silent `cargo test` needs.
    /// Below the no-progress abort matters because that abort only fires when
    /// keepalives *fail*: the case this cap exists for is a remote that keeps
    /// acking them while its shell never answers, which the abort cannot see.
    #[test]
    fn the_home_probe_budget_stays_under_both_transport_thresholds() {
        use super::super::transport::{NO_PROGRESS_ABORT, TRANSPORT_WALL_CAP};
        assert!(
            HOME_PROBE_CAP < TRANSPORT_WALL_CAP,
            "a one-line probe must not inherit the silent-compile allowance",
        );
        assert!(
            HOME_PROBE_CAP < NO_PROGRESS_ABORT,
            "a keepalive-acking remote that never answers must still fail fast",
        );
    }

    /// A non-zero exit means the remote shell never got far enough to print a
    /// HOME — caching whatever bytes came back would poison every later path
    /// computation for that machine, so it must be an error even if stdout
    /// happens to look plausible.
    #[test]
    fn rejects_home_when_the_probe_exits_non_zero() {
        let err = validate_home("/home/agent", 1).expect_err("non-zero exit must not be accepted");
        assert!(
            err.contains("Remote HOME probe exited with status 1"),
            "expected the exit-status error, got: {err}",
        );
    }

    /// `HOME` unset on the SSH session yields empty (or whitespace-only)
    /// output; an empty HOME would silently turn every `{home}/...` path into a
    /// relative one, so reject it up front.
    #[test]
    fn rejects_home_that_is_empty_or_whitespace() {
        for raw in ["", "   ", "\n", " \t\r\n "] {
            let err =
                validate_home(raw, 0).expect_err(&format!("expected an error for {raw:?}, got Ok"));
            assert_eq!(
                err, "Remote HOME is empty (HOME is not set on the SSH session)",
                "unexpected error text for {raw:?}",
            );
        }
    }

    /// Everything downstream joins onto this value, so a relative HOME would
    /// resolve against whatever cwd the channel happened to land in.
    #[test]
    fn rejects_home_that_is_not_absolute() {
        let err = validate_home("home/agent", 0).expect_err("a relative HOME must not be accepted");
        assert_eq!(
            err,
            "Remote HOME is not an absolute path (got 'home/agent')"
        );
    }

    /// The happy path: a clean exit and an absolute path, returned with the
    /// surrounding whitespace the shell may have added stripped off.
    #[test]
    fn accepts_absolute_home_and_trims_surrounding_whitespace() {
        assert_eq!(
            validate_home("  /home/agent\n", 0),
            Ok("/home/agent".to_string())
        );
    }
}
