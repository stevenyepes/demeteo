//! The queue of worktree directories Demeteo could not delete.
//!
//! Teardown's own backoff covers the transient case — a handle released
//! within a couple of seconds. What it cannot cover is a holder that
//! outlives the process: a scanner mid-scan, an indexer, an editor the
//! user left open on a file in the tree. Those failures are still
//! transient in the sense that they clear on their own; they just clear
//! on a timescale no in-process budget can wait for.
//!
//! So a failed delete is recorded, and the record is the contract:
//!
//! * **It is retried** — [`WorktreeCleanupQueuePort::due_for_retry`] is
//!   the startup sweep's work list.
//! * **It is bounded** — after [`MAX_AUTO_ATTEMPTS`] the entry stops
//!   being swept. Automatic retries cannot fix a structural cause, and a
//!   queue that keeps trying forever is how leftovers stay invisible.
//! * **It is visible** — [`LeakedWorktree::needs_attention`] is what a
//!   notice keys on, and [`WorktreeCleanupQueuePort::list`] returns
//!   entries whether or not they are still being swept. Silent
//!   accumulation is the defect this exists to prevent, so giving up
//!   quietly would reproduce it exactly.
//!
//! Nothing removes an entry except a confirmed deletion
//! ([`WorktreeCleanupQueuePort::record_success`]) or the user asking for
//! another round ([`WorktreeCleanupQueuePort::reset_attempts`]).

use serde::{Deserialize, Serialize};

/// Automatic attempts a leftover path gets before it is left to a human.
///
/// Sweeps run at most once per app start, so this is five separate
/// sessions — generous against every holder that releases on its own,
/// and short of pretending that a sixth identical attempt will behave
/// differently from the fifth.
pub const MAX_AUTO_ATTEMPTS: u32 = 5;

/// One directory that outlived its teardown, as the queue holds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeakedWorktree {
    /// Which filesystem `path` is on. `LOCAL_MACHINE`
    /// ([`crate::domain::ids::LOCAL_MACHINE`]) for the desktop host.
    pub machine_id: String,
    /// Normalized by [`normalize_queue_path`] before it is stored.
    pub path: String,
    /// What the leftover belonged to, for the notice. Absent when the
    /// caller had no feature in hand.
    pub feature_id: Option<String>,
    /// The most recent failure, not the first: a path that starts
    /// failing for a new reason should report the reason it fails now.
    pub last_error: String,
    /// Every attempt this entry has ever had, across resets.
    pub attempts: u32,
    /// The `attempts` value the current automatic budget counts from.
    pub auto_attempt_base: u32,
    pub first_enqueued_at: i64,
    pub last_attempt_at: i64,
}

impl LeakedWorktree {
    /// Attempts against the current budget, which
    /// [`WorktreeCleanupQueuePort::reset_attempts`] restarts.
    pub fn auto_attempts(&self) -> u32 {
        self.attempts.saturating_sub(self.auto_attempt_base)
    }

    /// Out of automatic attempts — the entry is now the user's to
    /// resolve, and a notice should say so.
    pub fn needs_attention(&self) -> bool {
        self.auto_attempts() >= MAX_AUTO_ATTEMPTS
    }
}

/// One failed deletion, as reported by teardown or by a sweep.
pub struct CleanupFailure<'a> {
    pub machine_id: &'a str,
    pub path: &'a str,
    pub feature_id: Option<&'a str>,
    pub error: &'a str,
    pub now: i64,
}

/// Fold the spellings of one directory onto one key.
///
/// Trailing separators only: `remove_dir_all` accepts a path with or
/// without one, so teardown and a later sweep can easily disagree about
/// it and split one stuck directory into two rows that each retry
/// independently.
///
/// Both separators are trimmed on every platform rather than under a
/// `cfg`, so the Windows behaviour is the behaviour a Linux test
/// exercises. The cost is a POSIX filename ending in a literal backslash,
/// which no path Demeteo constructs can produce.
///
/// A root (`/`, `C:\`) is returned unchanged — trimming it to nothing, or
/// to a bare drive letter that resolves to the process's current
/// directory on Windows, would be worse than not folding at all.
pub fn normalize_queue_path(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() || trimmed.ends_with(':') {
        path.to_string()
    } else {
        trimmed.to_string()
    }
}

pub trait WorktreeCleanupQueuePort: Send + Sync {
    /// Record a directory that could not be deleted, and return the entry
    /// as it now stands so the caller can act on
    /// [`LeakedWorktree::needs_attention`] without a second query.
    ///
    /// Idempotent on `(machine_id, normalized path)`: a repeat report
    /// bumps `attempts` and replaces `last_error` on the existing row
    /// rather than adding one. `first_enqueued_at` keeps the value from
    /// the first report — how long a leftover has been stuck is the part
    /// a user judges it by.
    fn record_failure(&self, failure: CleanupFailure<'_>) -> Result<LeakedWorktree, String>;

    /// Forget a path that is confirmed gone. Returns whether an entry was
    /// actually removed, so a sweep can report what it cleared. Deleting
    /// a path that was never queued is success, not an error — teardown
    /// calls this on the normal path too.
    fn record_success(&self, machine_id: &str, path: &str) -> Result<bool, String>;

    /// Every entry for `machine_id`, longest-stuck first, including the
    /// ones past [`MAX_AUTO_ATTEMPTS`]. This is what the user is shown.
    fn list(&self, machine_id: &str) -> Result<Vec<LeakedWorktree>, String>;

    /// The subset [`list`](Self::list) returns that is still under the
    /// automatic cap, longest-stuck first. This is what a sweep retries.
    fn due_for_retry(&self, machine_id: &str) -> Result<Vec<LeakedWorktree>, String>;

    /// Put an entry that ran out of attempts back into the sweep, at the
    /// user's request. The one way out of `needs_attention` short of the
    /// directory disappearing; without it, giving up would be permanent.
    fn reset_attempts(&self, machine_id: &str, path: &str, now: i64) -> Result<(), String>;
}
