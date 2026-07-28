//! What a `sequence` step has to show for itself so far, and what one task
//! added to it.
//!
//! Three things accumulate across a task list and belong to the *step*, not
//! to any task: the artifact references it hands downstream, the declared
//! deliverables some task has satisfied, and the tasks whose commits are
//! pinned. They used to travel as three `&mut` out-params threaded through
//! `run_tasks_loop` into `run_one_task` — which meant the loop had no way to
//! learn what a single task contributed except to snapshot the length of one
//! and clone the other, run the task, and diff. That snapshot-and-diff was
//! an out-parameter protocol reimplementing a return value, and it had to be
//! kept in step with a mutation happening two frames down.
//!
//! So a task now *returns* a [`TaskContribution`] and the step folds it into
//! a [`StepTally`]. The accumulation rules — refs append in landed order,
//! declarations are a set — live here, synchronously, next to the resume
//! payload they are seeded from and the checkpoint payload they produce.

use std::collections::HashSet;

use crate::domain::models::CheckpointProduced;

/// A task this attempt finished *and committed*, with the worktree HEAD its
/// commit produced. When a later task fails, the caller resets the worktree
/// to the last entry's `sha` (discarding the failed task's debris, including
/// any commits its agent made itself) and merges the prefix to the feature
/// branch, so the completed tasks' work — already paid for — survives the
/// failure and the retry runs only the remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandedTask {
    pub id: String,
    pub sha: String,
}

/// What one finished task emitted.
///
/// Returned rather than written through `&mut`, because it is also the
/// checkpoint payload recorded alongside that task's id: the two must name
/// the same task's output or a resume attributes one task's artifacts to
/// another's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskContribution {
    /// Artifact-store references this task's declared artifacts resolved to.
    pub artifact_refs: Vec<String>,
    /// Names of the `StepConfig::artifacts` declarations this task
    /// satisfied. A declaration missing from *one* task is not a failure —
    /// only one task in a list may be the one that writes the report — so
    /// this is evidence for the step-wide judgement, never a verdict.
    pub satisfied_decls: Vec<String>,
}

impl TaskContribution {
    /// This task's output as the checkpoint payload written with its id.
    pub fn produced(&self) -> CheckpointProduced {
        CheckpointProduced {
            artifact_refs: self.artifact_refs.clone(),
            satisfied_decls: self.satisfied_decls.clone(),
        }
    }
}

/// The step-scoped accumulators for one `sequence` attempt.
///
/// It does not start empty on a resume: a task that landed under an earlier
/// attempt contributed to all of these, and its contribution survives only
/// in the checkpoint's [`CheckpointProduced`] payload. Seeding from that row
/// is what lets a resumed step be judged on the same evidence as one that
/// ran the list, rather than exempted from the judgement.
#[derive(Debug)]
pub struct StepTally {
    artifact_refs: Vec<String>,
    satisfied_decls: HashSet<String>,
    landed: Vec<LandedTask>,
}

impl StepTally {
    /// Start an attempt from what a previous one already banked.
    ///
    /// `None` — no checkpoint, or a pre-V36 row that cannot say — starts
    /// empty. That is not the same claim as "produced nothing", and the
    /// caller has to keep the two apart; this type only holds what it was
    /// told.
    ///
    /// `landed` is deliberately *not* seeded: it names the tasks **this**
    /// attempt committed, and the prefix it drives is this attempt's alone.
    pub fn resuming(produced: Option<&CheckpointProduced>) -> Self {
        let mut tally = Self {
            artifact_refs: Vec::new(),
            satisfied_decls: HashSet::new(),
            landed: Vec::new(),
        };
        if let Some(produced) = produced {
            tally.artifact_refs.extend(produced.artifact_refs.clone());
            tally
                .satisfied_decls
                .extend(produced.satisfied_decls.iter().cloned());
        }
        tally
    }

    /// Fold one finished task's output into the step's totals.
    ///
    /// References append in landed order — downstream consumers read them as
    /// a list — while declarations are a set: two tasks may each write the
    /// same declared deliverable, and the step only ever asks whether *some*
    /// task did.
    pub fn fold(&mut self, contribution: TaskContribution) {
        self.artifact_refs.extend(contribution.artifact_refs);
        self.satisfied_decls.extend(contribution.satisfied_decls);
    }

    /// Record a task whose commit this attempt pinned.
    pub fn land(&mut self, task: LandedTask) {
        self.landed.push(task);
    }

    /// The tasks this attempt committed, in landed order. The last one is
    /// the prefix anchor a mid-list failure resets to.
    pub fn landed(&self) -> &[LandedTask] {
        &self.landed
    }

    /// Everything the step hands downstream, in landed order.
    pub fn artifact_refs(&self) -> &[String] {
        &self.artifact_refs
    }

    /// Did any task satisfy this declaration?
    pub fn satisfies(&self, decl: &str) -> bool {
        self.satisfied_decls.contains(decl)
    }

    /// The step-wide payload for a checkpoint write.
    ///
    /// Used by the mid-list failure path, which re-records the whole prefix
    /// to close the gap left by a task whose own checkpoint write failed.
    /// The store's write unions and deduplicates, so re-stating what is
    /// already there costs nothing.
    pub fn produced(&self) -> CheckpointProduced {
        CheckpointProduced {
            artifact_refs: self.artifact_refs.clone(),
            satisfied_decls: self.satisfied_decls.iter().cloned().collect(),
        }
    }

    /// Take in references swept from the artifact store rather than earned
    /// by a task this attempt ran.
    ///
    /// The pre-V36 compatibility path only: a resumed whole list whose
    /// checkpoint cannot say what it produced would otherwise starve its
    /// consumers of the refs. Not equivalent to a payload — the store has no
    /// attempt dimension, so a sweep also names files a rolled-back attempt
    /// wrote — which is why it is spelled differently from [`Self::fold`].
    pub fn recover_refs(&mut self, refs: Vec<String>) {
        self.artifact_refs.extend(refs);
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/progress.rs"]
mod tests;
