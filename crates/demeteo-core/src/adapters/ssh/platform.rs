//! Remote platform resolution: the `uname -s` probe, the rules an answer has
//! to satisfy, and the per-machine cache in front of both. Shaped after the
//! sibling `home` module, whose `$HOME` concern this mirrors exactly — same
//! channel round-trip, same never-invalidated cache, same reason.

use super::probe::probe_over_channel;
use super::retry::SshFailure;
use super::session::SessionPool;
use crate::domain::models::Platform;
use ssh2::Session;

fn probe_platform_over_channel(session: &Session) -> Result<Platform, SshFailure> {
    let (raw, exit_code) = probe_over_channel(session, "uname -s", "platform probe")?;
    validate_platform(&raw, exit_code).map_err(SshFailure::answered)
}

/// Validate the raw bytes of a `uname -s` probe. Split from the channel
/// round-trip so the rules are testable without a live socket.
///
/// An unrecognised `sysname` is an error rather than a fallback to
/// [`Platform::Linux`]. Remote execution is Linux-only (R2,
/// `docs/REMOTE_EXECUTION.md`), so "not Linux and not Darwin" means the machine
/// is not the one the caller believes it is registered as — and defaulting
/// would hand every downstream POSIX assumption to a host that was just
/// observed not to be one.
fn validate_platform(raw: &str, exit_code: i32) -> Result<Platform, String> {
    if exit_code != 0 {
        return Err(format!(
            "Remote platform probe exited with status {exit_code}; the SSH session may be denying shell access"
        ));
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(
            "Remote platform probe printed nothing (`uname -s` is unavailable on this host)"
                .to_string(),
        );
    }
    Platform::from_uname(trimmed)
        .ok_or_else(|| format!("Remote platform is not a supported target (uname -s: '{trimmed}')"))
}

impl SessionPool {
    /// Resolve the remote platform, serving it from `platform_cache` when we've
    /// already paid for the round-trip on this machine.
    ///
    /// Blocking, like [`SessionPool::resolve_home`], and reachable only from
    /// inside `spawn_blocking` for the same reason.
    pub(super) fn resolve_platform(&self, machine_id: &str) -> Result<Platform, SshFailure> {
        if let Ok(cache) = self.platform_cache.lock() {
            if let Some(platform) = cache.get(machine_id) {
                return Ok(*platform);
            }
        }

        let sftp_sess = self.get(machine_id).map_err(SshFailure::never_reached)?;
        let platform = probe_platform_over_channel(&sftp_sess.session)?;

        if let Ok(mut cache) = self.platform_cache.lock() {
            cache.insert(machine_id.to_string(), platform);
        }
        Ok(platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_two_kernels_a_remote_may_be() {
        assert_eq!(validate_platform("Linux\n", 0), Ok(Platform::Linux));
        assert_eq!(validate_platform("Darwin\n", 0), Ok(Platform::MacOS));
    }

    /// A shell that could not run `uname` still exits, and its stdout may carry
    /// anything the profile echoed. Caching that as a platform would make every
    /// later POSIX decision for this machine rest on a banner.
    #[test]
    fn rejects_a_platform_when_the_probe_exits_non_zero() {
        let err = validate_platform("Linux", 127).expect_err("non-zero exit must not be accepted");
        assert!(
            err.contains("Remote platform probe exited with status 127"),
            "expected the exit-status error, got: {err}",
        );
    }

    #[test]
    fn rejects_a_probe_that_printed_nothing() {
        for raw in ["", "   ", "\n"] {
            validate_platform(raw, 0)
                .expect_err(&format!("expected an error for {raw:?}, got a platform"));
        }
    }

    /// The silent-Linux trap: an unknown `sysname` must fail loudly rather than
    /// inherit the only platform a remote is allowed to be.
    #[test]
    fn refuses_to_call_an_unknown_kernel_linux() {
        for sysname in ["FreeBSD", "SunOS", "MINGW64_NT-10.0"] {
            let err = validate_platform(sysname, 0)
                .expect_err(&format!("{sysname} must not resolve to a platform"));
            assert!(
                err.contains(sysname),
                "the error must name what the remote said, got: {err}",
            );
        }
    }
}
