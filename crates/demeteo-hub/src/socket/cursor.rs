//! What an ack may name.

/// The instance's cursor after an event at offset `stored` was written, or
/// dropped as one the mirror already held, when the cursor was `before`.
///
/// The cursor is the highest offset `o` such that every event received from
/// the instance with an offset at or below `o` is durably stored. An instance
/// resumes from what it was acked, so the cursor never moves backwards: an
/// event resent from below it is stored if it was missing and leaves the
/// cursor where it is.
///
/// It is not `n, n + 1`. An offset is the desktop's table-global
/// `run_events.id`, one sequence across every Feature, and an event whose
/// kind is outside the protocol's allowlist is dropped on the instance before
/// it is sent. The offsets that arrive are sparse by design, and a cursor
/// that waited for `before + 1` would stall at the first gap and stay there.
///
/// The definition comes to the highest offset stored, which is all this
/// computes, because of two things [`session::run`](super::session::run) does
/// that this function cannot see: a frame is processed to completion before
/// the next is read, and a write that fails ends the session with no ack. So
/// no event is received and left unstored below one that was stored. A
/// session that read ahead of its writes, or carried on past a failed one,
/// would make this arithmetic wrong without changing a line of it.
pub fn advanced(before: u64, stored: u64) -> u64 {
    before.max(stored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cursor_follows_the_highest_offset_across_gaps_and_never_moves_back() {
        assert_eq!(advanced(0, 0), 0);
        assert_eq!(advanced(0, 3), 3);
        assert_eq!(advanced(3, 7), 7);
        assert_eq!(advanced(7, 20), 20);
        assert_eq!(advanced(20, 20), 20);
        assert_eq!(advanced(20, 5), 20);
        assert_eq!(advanced(20, 0), 20);
        assert_eq!(advanced(u64::MAX, 1), u64::MAX);
    }
}
