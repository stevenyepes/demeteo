//! When a detached run's waiter gives up on a feature that never ends.
//!
//! The task that waits for a run to reach a terminal state is also the only
//! thing that pushes its branch and opens its PR, so giving up is not free:
//! the engine keeps driving the feature, and whatever it finishes afterwards
//! is never published. The backstop exists for a feature wedged with nothing
//! moving it, and it must not fire on one that is merely waiting.
//!
//! A run parked on a question for a person is the case that looks wedged and
//! is not. Its wait is as long as the person takes, so that time is not
//! counted — a run approved on the fifth day publishes like one approved in
//! the first hour.

use std::time::Duration;

/// How long a run may go without reaching a terminal state, not counting time
/// spent waiting on a person.
pub const MAX_UNATTENDED_WAIT: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WaitBackstop {
    unattended: Duration,
}

impl WaitBackstop {
    /// Account for `elapsed` since the previous observation, and say whether
    /// the waiter should give up.
    ///
    /// `waiting_on_person` describes the whole of `elapsed`. The caller polls
    /// every few seconds against a limit of a day, so attributing an interval
    /// to the state seen at its end is well inside the noise.
    pub fn observe(&mut self, elapsed: Duration, waiting_on_person: bool) -> bool {
        if !waiting_on_person {
            self.unattended = self.unattended.saturating_add(elapsed);
        }
        self.unattended > MAX_UNATTENDED_WAIT
    }
}

#[cfg(test)]
#[path = "../../tests/domain/wait_backstop.rs"]
mod tests;
