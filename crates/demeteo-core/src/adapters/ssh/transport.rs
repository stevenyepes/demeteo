//! Keepalive-aware drain policy for SSH streams: the boundary between "quiet
//! but alive" and "wedged connection". Kept free of `Session` state so the
//! policy is unit-testable without a live socket.

use ssh2::Session;
use std::io::{ErrorKind, Read};
use std::time::{Duration, Instant};

/// Outer wall-clock bound for draining a single one-shot SSH command. The
/// keepalive-aware loop in [`drain_stream`] lets a command stay silent for as
/// long as it needs without the session's 10s blocking-call timeout aborting
/// it (that was the "`cargo test` compiles silently for >10s → prepare command
/// spuriously fails" bug — see the conformance suite's long-silent-command
/// clause). We still cap total drain time so a genuinely wedged remote can't
/// hang a step forever. This is a transport backstop, not a per-command tuning
/// knob — finer wall-clock limits and cooperative cancellation belong to the
/// caller's timeout layer.
pub(super) const TRANSPORT_WALL_CAP: Duration = Duration::from_secs(30 * 60);

/// How long a drain may go with **no sign of life** — no bytes read *and* no
/// successful keepalive round-trip — before the transport is declared dead and
/// the drain aborts. This is the difference between "quiet but alive" and
/// "connection wedged": a silent-but-healthy command (a `cargo test` compiling
/// in silence) keeps answering keepalives every ~30s, so its life clock is
/// continually reset and it survives up to [`TRANSPORT_WALL_CAP`]. A
/// black-holed connection stops acking keepalives, so this trips in ~2 min
/// instead of keepalive-looping to the full 30-minute cap — which used to
/// freeze not just the step but every SSH op queued behind the pooled session.
/// Deliberately larger than the 30s keepalive interval so a single transient
/// blip never false-positives, and smaller than the wall cap so it fails fast.
pub(super) const NO_PROGRESS_ABORT: Duration = Duration::from_secs(120);

/// Has a silent drain crossed from "quiet but alive" into "the transport is
/// dead"? `since_last_life` is how long since we last saw *either* bytes on the
/// wire *or* a keepalive round-trip. Extracted (and kept free of `Session`) so
/// the boundary is unit-testable without a live socket.
fn no_progress_expired(since_last_life: Duration) -> bool {
    since_last_life >= NO_PROGRESS_ABORT
}

/// Tag `msg` as a *transport/connection* failure (the machine could not be
/// reached or the channel broke) rather than a *command* failure (it ran and
/// exited non-zero). Callers distinguish the two via
/// [`crate::ports::execution::TRANSPORT_ERROR_PREFIX`] (C0.2, D3) — e.g. the
/// verifier routes a transport failure to `Infrastructure` (non-retryable)
/// instead of a `Verdict` that would pointlessly re-run a failing build.
pub(super) fn transport_err(msg: impl std::fmt::Display) -> String {
    format!("{}{}", crate::ports::execution::TRANSPORT_ERROR_PREFIX, msg)
}

/// Drain `reader` (an ssh2 channel or its stderr stream) to EOF into
/// `buf_out`, tolerating the session's blocking-call timeout the way the
/// interactive [`super::interactive::RemoteChannelHandle`]`::try_read` path
/// does: a `TimedOut` /
/// `WouldBlock` read is **not** end-of-stream — libssh2 aborts a blocking read
/// the moment a keepalive comes due (~30s after handshake) even while the
/// command is alive and simply quiet. Send the keepalive it's waiting on and
/// retry, so a long silent compile drains to real EOF instead of failing with
/// "Timed out waiting on socket". `deadline` bounds the whole drain so a
/// wedged remote is still killable. Bytes are accumulated raw and decoded once
/// by the caller — decoding per chunk could split a multibyte UTF-8 sequence.
pub(super) fn drain_stream<R: Read>(
    reader: &mut R,
    session: &Session,
    buf_out: &mut Vec<u8>,
    deadline: Instant,
    what: &str,
) -> Result<(), String> {
    // The liveness signal (`keepalive_send`) and the clock (`Instant::now`) are
    // injected so the drain policy below is unit-testable without a live socket
    // — see `drain_loop`'s tests, which exercise the dead-connection abort that
    // can't otherwise be driven deterministically.
    drain_loop(
        reader,
        || session.keepalive_send().is_ok(),
        Instant::now,
        buf_out,
        deadline,
        what,
    )
}

/// The transport-agnostic core of [`drain_stream`]. `keepalive_ok` reports
/// whether a liveness probe succeeded (on a real session, whether
/// `keepalive_send` returned `Ok`); `clock` yields the current instant. Both
/// are parameters purely so tests can feed a scripted reader, a
/// forced-failing keepalive, and a controllable clock — production always
/// passes the real session probe and `Instant::now`.
fn drain_loop<R, K, C>(
    reader: &mut R,
    mut keepalive_ok: K,
    mut clock: C,
    buf_out: &mut Vec<u8>,
    deadline: Instant,
    what: &str,
) -> Result<(), String>
where
    R: Read,
    K: FnMut() -> bool,
    C: FnMut() -> Instant,
{
    let mut chunk = [0u8; 8192];
    // Last moment the transport showed life: either bytes arrived or a
    // keepalive round-tripped. A merely quiet command keeps this fresh (every
    // keepalive is answered); a wedged connection lets it go stale, which is
    // how we tell the two apart without killing healthy silent commands.
    let mut last_life = clock();
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                buf_out.extend_from_slice(&chunk[..n]);
                last_life = clock();
            }
            Err(e) if matches!(e.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                let now = clock();
                if now >= deadline {
                    return Err(transport_err(format!(
                        "Timed out after the transport wall cap ({}s) waiting for {}",
                        TRANSPORT_WALL_CAP.as_secs(),
                        what
                    )));
                }
                // A blocking read times out every ~10s while a command is
                // simply quiet (see `ssh_util::connect`'s `set_timeout`). The
                // keepalive tells us whether the *transport* is still alive:
                // on a live session it round-trips (`Ok`) and we refresh the
                // life clock; on a black-holed one it errors (or its socket
                // write times out), the clock goes stale, and we abort once it
                // crosses `NO_PROGRESS_ABORT` instead of looping to the wall
                // cap and freezing every SSH op behind the pooled session.
                if keepalive_ok() {
                    last_life = now;
                } else if no_progress_expired(now.duration_since(last_life)) {
                    return Err(transport_err(format!(
                        "Connection appears dead: no data and no keepalive response for {}s while waiting for {}",
                        NO_PROGRESS_ABORT.as_secs(),
                        what
                    )));
                }
            }
            Err(e) => return Err(transport_err(format!("Failed to read {}: {}", what, e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fast-abort window is only correct if it sits strictly between the
    /// keepalive interval and the wall cap: larger than the interval so a
    /// quiet-but-alive command (which answers a keepalive every ~30s) never
    /// trips it, and smaller than the wall cap so a dead connection fails fast
    /// instead of hanging the pipeline for the full 30 minutes. Lock that
    /// ordering so a future tweak to any one constant can't silently break it.
    #[test]
    fn no_progress_abort_sits_between_keepalive_and_wall_cap() {
        // Keepalive interval configured on the session (`set_keepalive(true, 30)`).
        const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
        assert!(
            NO_PROGRESS_ABORT > KEEPALIVE_INTERVAL,
            "must outlast a keepalive cycle so silent-but-alive commands survive",
        );
        assert!(
            NO_PROGRESS_ABORT < TRANSPORT_WALL_CAP,
            "must fire before the wall cap so a dead connection fails fast",
        );
    }

    /// The boundary is inclusive at exactly `NO_PROGRESS_ABORT` and never trips
    /// before it — so a healthy session whose keepalives keep resetting the
    /// life clock (`since_last_life` stays near zero) is never declared dead.
    #[test]
    fn no_progress_expires_only_at_or_past_the_window() {
        assert!(!no_progress_expired(Duration::from_secs(0)));
        assert!(!no_progress_expired(
            NO_PROGRESS_ABORT - Duration::from_millis(1)
        ));
        assert!(no_progress_expired(NO_PROGRESS_ABORT));
        assert!(no_progress_expired(
            NO_PROGRESS_ABORT + Duration::from_secs(300)
        ));
    }

    /// A `Read` that always reports a timed-out read — i.e. a connection that
    /// is silent forever, the shape both a quiet-but-alive command and a
    /// black-holed transport present to `drain_loop`. What tells them apart is
    /// the keepalive result, not the reader.
    struct SilentReader;
    impl Read for SilentReader {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::from(ErrorKind::WouldBlock))
        }
    }

    /// A clock that advances a fixed `step` on every read, starting at a fixed
    /// base — deterministic stand-in for `Instant::now` so the abort window can
    /// be crossed without real sleeping.
    fn stepping_clock(step: Duration) -> impl FnMut() -> Instant {
        let base = Instant::now();
        let mut ticks = 0u32;
        move || {
            let t = base + step * ticks;
            ticks += 1;
            t
        }
    }

    /// The regression this whole change exists for: when the transport is
    /// black-holed — silent reads *and* failing keepalives — `drain_loop` must
    /// abort with the dead-connection error once `NO_PROGRESS_ABORT` elapses,
    /// instead of looping to the 30-minute wall cap and freezing every SSH op
    /// queued behind the pooled session. Before keepalive was actually enabled
    /// on pooled sessions this path was unreachable in production; this test
    /// drives it directly.
    #[test]
    fn drain_loop_aborts_when_keepalive_fails_and_no_bytes_arrive() {
        let mut buf = Vec::new();
        // Wall cap far in the future so the abort we assert is the no-progress
        // one, not the wall-cap timeout. Clock advances 10s per read (the real
        // blocking-read timeout cadence).
        let deadline = Instant::now() + TRANSPORT_WALL_CAP;
        let err = drain_loop(
            &mut SilentReader,
            || false, // keepalive always fails → transport is dead
            stepping_clock(Duration::from_secs(10)),
            &mut buf,
            deadline,
            "command stdout",
        )
        .expect_err("a black-holed transport must abort, not hang");
        assert!(
            err.contains("Connection appears dead"),
            "expected the no-progress abort, got: {err}",
        );
        assert!(buf.is_empty());
    }

    /// The mirror property: a quiet-but-*alive* command (silent reads but
    /// keepalives keep succeeding) must never be declared dead — its life clock
    /// is reset every probe. Here EOF eventually arrives and the drain returns
    /// `Ok`, having spanned well past `NO_PROGRESS_ABORT` in clock time.
    #[test]
    fn drain_loop_survives_silence_while_keepalives_succeed() {
        /// Times out `n` times (crossing the abort window in clock terms),
        /// then reports EOF.
        struct SilentThenEof(u32);
        impl Read for SilentThenEof {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0 == 0 {
                    Ok(0)
                } else {
                    self.0 -= 1;
                    Err(std::io::Error::from(ErrorKind::WouldBlock))
                }
            }
        }

        let mut buf = Vec::new();
        let deadline = Instant::now() + TRANSPORT_WALL_CAP;
        // 30 timeouts × 10s = 300s of silence, > NO_PROGRESS_ABORT (120s).
        let out = drain_loop(
            &mut SilentThenEof(30),
            || true, // keepalive always succeeds → still alive
            stepping_clock(Duration::from_secs(10)),
            &mut buf,
            deadline,
            "command stdout",
        );
        assert!(
            out.is_ok(),
            "a silent-but-alive command must drain to EOF, got: {out:?}",
        );
    }
}
