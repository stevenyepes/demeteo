//! What a runner restart does to a run it finds unfinished.
//!
//! A run may be re-attached after a restart only so many times. The budget is
//! a guard against a crash-looping host re-spending agent turns on one run
//! forever, so what it counts is *restarts that interrupted work* — not
//! restarts. A run parked on a question for a person has no work in flight; a
//! host that is shut down, redeployed or rebooted while it waits costs that
//! run nothing, and a run may wait for days.
//!
//! Charging every restart parked exactly those runs as `failed`: nothing was
//! lost, the engine still drove the feature to the end once the person
//! answered, and the push and PR — which only the re-attached run performs —
//! never happened.

/// How many work-interrupting restarts a run survives.
pub const REBOOT_RETRY_BUDGET: i64 = 5;

/// Run-row statuses that say a task was driving the run when the process
/// ended. Every other status is either terminal or a park something else
/// resumes (`needs-credentials`, by the credential arriving).
pub fn left_unfinished_by_restart(run_status: &str) -> bool {
    matches!(run_status, "running" | "pending" | "interrupted")
}

/// Whether the restart found the run with work in flight.
///
/// `pending_question_asked_at` is the `created_at` of the newest undecided
/// gate row of the run's feature, `process_started_at` a reading of the same
/// clock taken before the engine's own restart reconciliation ran.
///
/// The ordering is the whole test. The engine's reconciliation turns an
/// interrupted step into a question of its own ("may this resume?"), so by
/// the time anyone can ask, a run that was working and a run that was waiting
/// both show an undecided gate. Only their age tells them apart: a question
/// asked by an earlier process was already being waited on when this one
/// started, and one asked since is this restart's own.
///
/// Both ways this can be wrong charge a restart that should have been free,
/// never the reverse: a clock stepped backwards across the reboot makes an
/// old question look new, and no question at all is read as work.
pub fn restart_interrupted_work(
    pending_question_asked_at: Option<i64>,
    process_started_at: i64,
) -> bool {
    !pending_question_asked_at.is_some_and(|asked_at| asked_at < process_started_at)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartDisposition {
    /// Re-attach without touching the count.
    Resume,
    /// Re-attach, and count this restart against the budget.
    ResumeCharged,
    /// The budget is spent: record the run as failed and do not re-attach.
    Park,
}

/// What to do with an unfinished run that has already been charged
/// `resume_count` restarts.
///
/// A free restart re-attaches even on a spent budget. The budget stops work
/// from being repeated, and a run that is only waiting repeats none.
pub fn restart_disposition(resume_count: i64, interrupted_work: bool) -> RestartDisposition {
    if !interrupted_work {
        RestartDisposition::Resume
    } else if resume_count >= REBOOT_RETRY_BUDGET {
        RestartDisposition::Park
    } else {
        RestartDisposition::ResumeCharged
    }
}

#[cfg(test)]
#[path = "../../tests/domain/reboot_budget.rs"]
mod tests;
