// Tests extracted from `crates/demeteo-core/src/domain/sequence/outcome.rs` (mirrored-tests convention). `super` = that module.

use super::*;

/// A rollback that did not happen leaves the failed attempt's commits on the
/// feature branch. Every failure path — including the verifier's — must say
/// so, naming the branch, or the user retries/ships against a branch state
/// they were told does not exist.
#[test]
fn a_failed_rollback_warns_and_names_the_branch() {
    let out = FailureDisposition::RollbackFailed.decorate("verdict: fail", "feature/x");
    assert!(out.starts_with("verdict: fail"), "{out}");
    assert!(out.contains("could NOT be rolled back"), "{out}");
    assert!(out.contains("feature/x"), "{out}");
}

#[test]
fn a_clean_rollback_notes_the_clean_retry() {
    let out = FailureDisposition::RolledBack.decorate("boom", "feature/x");
    assert!(out.starts_with("boom"), "{out}");
    assert!(out.contains("rolled back for a clean retry"), "{out}");
}

#[test]
fn a_landed_prefix_reports_progress_and_the_branch() {
    let out = FailureDisposition::PrefixLanded {
        landed: 2,
        total: 5,
    }
    .decorate("task 'task-3' failed", "feature/x");
    assert!(out.contains("2 of 5 tasks completed"), "{out}");
    assert!(out.contains("feature/x"), "{out}");
    assert!(out.contains("resume from the failed task"), "{out}");
}

/// The context prefix names the task that failed. It must not be attached to
/// a cancellation: the user stopping the run is not *about* whichever task
/// happened to be in flight when they did.
#[test]
fn context_prefixes_a_failure_but_never_a_cancellation() {
    assert_eq!(
        SequenceError::Failed("boom".into()).with_context("sequence task 'task-3'"),
        SequenceError::Failed("sequence task 'task-3': boom".into())
    );
    assert_eq!(
        SequenceError::Environmental("gone".into()).with_context("sequence task 'task-3'"),
        SequenceError::Environmental("sequence task 'task-3': gone".into())
    );
    assert_eq!(
        SequenceError::Cancelled.with_context("sequence task 'task-3'"),
        SequenceError::Cancelled
    );
}

/// A cancellation carries no message of its own, so `message()` says so
/// rather than inventing one — the telemetry row for a cancelled task must
/// not read as though something went wrong.
#[test]
fn only_a_real_failure_has_a_message() {
    assert_eq!(SequenceError::Cancelled.message(), None);
    assert_eq!(SequenceError::Failed("boom".into()).message(), Some("boom"));
}

/// `Display` is what reaches the stored error and the user. A cancellation
/// renders as the phrase the rest of the executor already uses for it.
#[test]
fn a_cancellation_renders_as_the_shared_phrase() {
    assert_eq!(
        SequenceError::Cancelled.to_string(),
        "Execution cancelled by user"
    );
}
