// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/mod.rs` (mirrored-tests convention). `super` = that module.

use super::*;

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
