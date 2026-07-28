// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/mod.rs` (mirrored-tests convention). `super` = that module.

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

/// The ordinary verdict failure: tasks ran this attempt, so the rollback
/// discards *their* work and the row goes back to what the attempt read.
#[test]
fn a_verdict_after_running_tasks_rewinds_the_checkpoint() {
    let resume = CheckpointResume::Merged {
        landed_ids: vec!["t-1".into()],
        produced: None,
    };
    assert!(matches!(
        verdict_disposition(false, &resume),
        CheckpointDisposition::RewindTo(_)
    ));
}

/// The case a rewind cannot terminate. No task ran, so the work the verdict
/// rejected *is* the checkpoint: putting it back hands the retry the same
/// zero-task attempt, the same verdict, and the same tree until the budget
/// is gone — with the feedback never reaching an agent.
#[test]
fn a_verdict_against_a_zero_task_resume_discards_the_checkpoint() {
    let resume = CheckpointResume::Merged {
        landed_ids: vec!["t-1".into(), "t-2".into()],
        produced: None,
    };
    assert!(matches!(
        verdict_disposition(true, &resume),
        CheckpointDisposition::Discard
    ));
}
