//! Releasing the dependency cache of a feature a `demeteo-runner` owns. See
//! [`crate::domain`]; *when* a cache may go at all is
//! [`cache_release`](crate::domain::cache_release)'s.
//!
//! The runner cannot learn on its own that such a feature's work has ended.
//! The git PAT it pushed and opened the PR with is erased when the run turns
//! terminal (docs/REMOTE_EXECUTION.md §6.2), which is before any PR merges, so
//! its own MR monitor never sees the merge; and a user's cleanup happens on the
//! laptop. The laptop does see both, but the cache lives beside the runner's
//! clone, which no laptop-side path reaches. So the laptop tells the runner
//! why (`release_feature_cache`), and the runner records that on its own row
//! before judging the cache, so that its startup sweep reaches the same verdict
//! later if this release is lost.

use serde::{Deserialize, Serialize};

/// Why the laptop considers a runner-owned feature finished. The wire value
/// of `release_feature_cache`'s `reason`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheReleaseReason {
    Merged,
    Closed,
    /// The user cleaned the feature up (archived or deleted it) on the laptop.
    Dismissed,
}

impl CacheReleaseReason {
    /// The reason a feature in `status` with its PR in `mr_state` gives. A
    /// settled PR outranks a cleanup: it is the fact the runner would have
    /// recorded itself had it been able to see it.
    pub fn of(status: &str, mr_state: Option<&str>) -> Option<Self> {
        match (mr_state, status) {
            (Some("merged"), _) => Some(Self::Merged),
            (Some("closed"), _) => Some(Self::Closed),
            (_, "archived" | "deleted") => Some(Self::Dismissed),
            _ => None,
        }
    }
}

/// What the runner writes to its own feature row before judging the cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerSettle {
    /// Record the PR in this state, exactly as the MR monitor records a poll.
    MrState(&'static str),
    /// Archive the row. The runner keeps it, and never deletes the branch:
    /// the laptop's cleanup policy governs the laptop's records only.
    Archive,
}

/// The runner's answer to `release_feature_cache` for a run in `run_status`.
///
/// Refused while the run row is `pending` or `running` — a run parked at a gate
/// is still `running` — because its driver may read the cache at any moment,
/// and archiving or completing its feature under it would race that driver.
pub fn runner_settle(
    run_status: &str,
    run_id: &str,
    reason: CacheReleaseReason,
) -> Result<RunnerSettle, String> {
    if matches!(run_status, "pending" | "running") {
        return Err(format!(
            "run {run_id} is still {run_status}; its dependency cache is in use"
        ));
    }
    Ok(match reason {
        CacheReleaseReason::Merged => RunnerSettle::MrState("merged"),
        CacheReleaseReason::Closed => RunnerSettle::MrState("closed"),
        CacheReleaseReason::Dismissed => RunnerSettle::Archive,
    })
}

/// The runner's refusal for a run that is absent or owned by another client —
/// one prefix for both, so ownership leaks no existence signal (MC-D2).
pub const NO_SUCH_RUN: &str = "no such run: ";

/// A release the laptop could not deliver, kept until a later reconcile can.
///
/// A cleanup dismisses the laptop's mirror of the run in the same click, so
/// after it nothing else on the laptop would remember that the runner still
/// holds the cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingRunnerRelease {
    pub machine_id: String,
    pub run_id: String,
    pub reason: CacheReleaseReason,
}

/// `queue` with `entry` in it, replacing any earlier entry for the same run:
/// the latest reason is what the laptop last knew.
pub fn with_pending(
    mut queue: Vec<PendingRunnerRelease>,
    entry: PendingRunnerRelease,
) -> Vec<PendingRunnerRelease> {
    queue.retain(|p| !(p.machine_id == entry.machine_id && p.run_id == entry.run_id));
    queue.push(entry);
    queue
}

/// Whether a release that failed with `error` could succeed on a later try.
/// Unreachable, too old to know the method, or still running all can; a run
/// the runner does not know or will not admit to owning never will.
pub fn worth_retrying(error: &str) -> bool {
    !error.starts_with(NO_SUCH_RUN)
}

/// Whether a retry that failed with `error` on `machine_id` is news, given the
/// machines `too_old` already records as not knowing the method.
///
/// A runner too old for `release_feature_cache` answers the same way on every
/// reconcile until it is upgraded, and its entry is kept for that upgrade
/// ([`worth_retrying`]), so it is said once per machine rather than once per
/// reconcile. Every other failure is said each time: it can change.
pub fn worth_logging(
    too_old: &mut std::collections::HashSet<String>,
    machine_id: &str,
    error: &str,
) -> bool {
    !error.starts_with("unknown method") || too_old.insert(machine_id.to_string())
}

#[cfg(test)]
#[path = "../../tests/domain/runner_cache_release.rs"]
mod tests;
