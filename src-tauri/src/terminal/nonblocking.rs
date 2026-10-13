//! Writing to an SSH terminal channel once its session is non-blocking.
//!
//! `start_ssh_session` switches the session to non-blocking before the drain
//! thread starts, so every later call on the channel can fail with
//! `WouldBlock` (libssh2's `EAGAIN`) when the socket or the remote window is
//! momentarily full. `Write::write_all` gives up on the first one and reports
//! nothing about how much it wrote, so a paste lost its tail silently, and a
//! resize that hit `EAGAIN` left xterm and the remote PTY at different widths.
//!
//! Never "flush" here: `ssh2::Channel::flush` is `libssh2_channel_flush_ex`,
//! which *discards* inbound data queued on the stream and not yet read. Called
//! after every keystroke, it deleted agent output before the drain thread could
//! read it, desyncing every diff-rendering TUI.

use std::io;
use std::thread;
use std::time::{Duration, Instant};

/// How long one terminal write or resize may wait out `WouldBlock` before it
/// is reported as failed. Long enough to ride out a burst of agent output
/// filling the window; short enough that a dead transport surfaces as an error
/// instead of a hung IPC call.
pub(crate) const WOULD_BLOCK_BUDGET: Duration = Duration::from_secs(5);

const WOULD_BLOCK_BACKOFF: Duration = Duration::from_millis(2);

/// Run `op` until it returns anything other than `WouldBlock`, or the budget
/// runs out (the last `WouldBlock` is then returned). `op` should take its
/// lock itself, so the drain thread can read between attempts.
pub(crate) fn retry_would_block<T>(
    budget: Duration,
    mut op: impl FnMut() -> io::Result<T>,
) -> io::Result<T> {
    let deadline = Instant::now() + budget;
    loop {
        match op() {
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(WOULD_BLOCK_BACKOFF);
            }
            other => return other,
        }
    }
}

/// `write_all` for a non-blocking sink: partial writes advance, `WouldBlock`
/// is retried within `budget`, and a zero-length write is an error rather than
/// a spin.
pub(crate) fn write_all_retrying(
    mut buf: &[u8],
    budget: Duration,
    mut write: impl FnMut(&[u8]) -> io::Result<usize>,
) -> io::Result<()> {
    let deadline = Instant::now() + budget;
    while !buf.is_empty() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let n = retry_would_block(remaining, || write(buf))?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "terminal channel accepted no bytes",
            ));
        }
        buf = &buf[n..];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn would_block() -> io::Error {
        io::Error::new(io::ErrorKind::WouldBlock, "EAGAIN")
    }

    #[test]
    fn a_write_interrupted_by_would_block_delivers_every_byte_in_order() {
        let mut sink = Vec::new();
        let mut calls = 0;
        write_all_retrying(b"hello world", WOULD_BLOCK_BUDGET, |chunk| {
            calls += 1;
            if calls % 2 == 1 {
                return Err(would_block());
            }
            let n = chunk.len().min(3);
            sink.extend_from_slice(&chunk[..n]);
            Ok(n)
        })
        .unwrap();
        assert_eq!(sink, b"hello world");
    }

    #[test]
    fn a_sink_that_never_drains_fails_once_the_budget_is_spent() {
        let err = write_all_retrying(b"x", Duration::from_millis(20), |_| Err(would_block()))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn a_zero_length_write_is_an_error_not_a_spin() {
        let err = write_all_retrying(b"x", WOULD_BLOCK_BUDGET, |_| Ok(0)).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::WriteZero);
    }

    #[test]
    fn a_hard_error_is_returned_without_retrying() {
        let mut calls = 0;
        let err = retry_would_block::<()>(WOULD_BLOCK_BUDGET, || {
            calls += 1;
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "gone"))
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert_eq!(calls, 1);
    }
}
