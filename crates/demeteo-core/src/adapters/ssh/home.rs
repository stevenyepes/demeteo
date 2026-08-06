//! Remote `$HOME` resolution: the `printf %s "$HOME"` probe, the rules a
//! usable answer has to satisfy, and the per-machine cache in front of both.
//! Kept with [`SessionPool`] (whose `home_cache` it owns the reads of) so the
//! whole HOME concern is one file.

use super::probe::probe_over_channel;
use super::retry::SshFailure;
use super::session::SessionPool;
use ssh2::Session;

/// Probe `$HOME` over a fresh channel on an already-connected session.
/// `printf %s` avoids trailing newlines and respects quoting.
fn probe_home_over_channel(session: &Session) -> Result<String, SshFailure> {
    let (raw, exit_code) = probe_over_channel(session, "printf %s \"$HOME\"", "HOME probe")?;
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
