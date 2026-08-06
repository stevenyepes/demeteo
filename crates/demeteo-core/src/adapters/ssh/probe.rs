//! One-line shell probes over a fresh channel, and the budget they share.
//!
//! The two callers (`home`, `platform`) ask the remote a one-word question and
//! interpret the answer themselves. Only the round-trip is here; every rule
//! about what a *usable* answer looks like stays in the module that owns the
//! concern, so this file has no opinion to go stale.

use super::retry::SshFailure;
use super::transport::{drain_stream, DrainBudget};
use ssh2::Session;
use std::time::Duration;

/// Wall-clock allowance for a probe, deliberately far below the transport's
/// 30-minute [`TRANSPORT_WALL_CAP`].
///
/// That cap is sized for a one-shot command that may legitimately go quiet for
/// half an hour — `cargo test` compiling in silence. A probe is a single
/// builtin on a non-login, non-interactive shell, so it sources no profile and
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
const PROBE_CAP: Duration = Duration::from_secs(60);

/// Run `cmd` over a fresh channel on an already-connected session and return
/// its raw stdout alongside its exit status. Callers pass a `printf`-style
/// command so the output carries no trailing newline of the shell's invention.
///
/// Graded for the retry loop the same way `command::exec_over_channel` is, and
/// for the same reason even though these commands have no side effects: a probe
/// runs on [`PROBE_CAP`], so retrying a failure that happened *during* the drain
/// would spend that budget once per attempt. Only the channel open — where
/// nothing has been spent and nothing has run — is retried. See `super::retry`.
///
/// The exit status is returned rather than checked here because a non-zero exit
/// is not uniformly fatal: it is the caller's question whether a shell that
/// failed still said something usable, and every caller today answers no.
pub(super) fn probe_over_channel(
    session: &Session,
    cmd: &str,
    label: &str,
) -> Result<(String, i32), SshFailure> {
    let mut channel = session.channel_session().map_err(|e| {
        SshFailure::never_reached(format!("Failed to open SSH channel for {label}: {e}"))
    })?;
    channel
        .exec(cmd)
        .map_err(|e| SshFailure::may_have_run(format!("Failed to exec {label} over SSH: {e}")))?;
    let mut raw_bytes = Vec::new();
    drain_stream(
        &mut channel,
        session,
        &mut raw_bytes,
        DrainBudget::starting_now(PROBE_CAP),
        &format!("{label} output"),
    )
    .map_err(SshFailure::may_have_run)?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    channel.wait_close().map_err(|e| {
        SshFailure::may_have_run(format!("Failed to wait for {label} channel: {e}"))
    })?;
    // ssh2's `wait_close` returns `Result<(), Error>`; the exit status is on a
    // separate method that returns `Result<i32, Error>` (0 on success, non-zero
    // on remote failure). Drain it so a broken shell session doesn't get cached
    // as a valid answer.
    let exit_code = channel.exit_status().map_err(|e| {
        SshFailure::may_have_run(format!("Failed to read {label} exit status: {e}"))
    })?;
    Ok((raw, exit_code))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The probe budget has to stay below both transport thresholds, and the
    /// reason differs for each. Below the wall cap is the whole point — a
    /// `printf` must not be allowed the half hour a silent `cargo test` needs.
    /// Below the no-progress abort matters because that abort only fires when
    /// keepalives *fail*: the case this cap exists for is a remote that keeps
    /// acking them while its shell never answers, which the abort cannot see.
    #[test]
    fn the_probe_budget_stays_under_both_transport_thresholds() {
        use super::super::transport::{NO_PROGRESS_ABORT, TRANSPORT_WALL_CAP};
        assert!(
            PROBE_CAP < TRANSPORT_WALL_CAP,
            "a one-line probe must not inherit the silent-compile allowance",
        );
        assert!(
            PROBE_CAP < NO_PROGRESS_ABORT,
            "a keepalive-acking remote that never answers must still fail fast",
        );
    }
}
