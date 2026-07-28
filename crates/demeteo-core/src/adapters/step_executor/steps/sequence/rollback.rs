//! Unwinding a `sequence` attempt that will not complete: salvage the prefix
//! that did land, put the feature branch back where it was, and report the
//! failure as the outcome the caller must return.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::sequence::checkpoint::CheckpointDisposition;
use crate::domain::sequence::outcome::{FailureDisposition, SequenceError};
use crate::domain::sequence::progress::StepTally;
use crate::domain::sequence::sha::Sha;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

use super::context::{RunTarget, StepCtx, StepSpend, StepWorktree};

impl ExecutionDriver {
    /// Merge the prefix this attempt completed to the feature branch and
    /// record it, so a retry runs only the remainder.
    ///
    /// `Some(total)` is how many tasks the checkpoint now claims — the
    /// count a `PrefixLanded` disposition reports. `None` means the merge
    /// did not land and the prefix is still on the step branch; the caller
    /// decides what that is worth.
    pub(crate) async fn salvage_landed_prefix(
        &self,
        step: StepCtx<'_>,
        target: RunTarget<'_>,
        wt: StepWorktree<'_>,
        tally: &StepTally,
    ) -> Option<usize> {
        if let Err(e) = self
            .checkpoint_landed_prefix(target.machine, wt, tally.landed())
            .await
        {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step.step_id(),
                error = %e,
                "sequence step: could not merge the completed task prefix; \
                 falling back to a full rollback, keeping the prefix pinned for \
                 the next attempt to restore"
            );
            return None;
        }

        // Durable (V32): record the landed prefix so the next
        // attempt — in this process or after a restart — runs
        // only the remainder. A write failure degrades to the
        // in-attempt count; the retry then re-runs tasks whose
        // agents will find (and be told about) the committed
        // work, which is the pre-V32 restart behavior.
        //
        // The task loop has usually recorded each of these
        // already; this write is what closes the gap for a
        // task whose own checkpoint failed. The anchor is
        // re-stamped rather than left alone because the merge
        // above just made it an *ancestor* of the feature
        // branch — which is precisely how the next attempt
        // tells "already merged, skip the ids" from "still
        // only on the step branch, restore it first".
        let landed_ids: Vec<String> = tally.landed().iter().map(|t| t.id.clone()).collect();
        let anchor = tally.landed().last().map(|t| t.sha.as_str());
        let step_id = step.step_id();
        // The same gap-closing for the produced payload: a
        // task whose own checkpoint write failed folded into
        // the tally anyway, and the failed task did not (it
        // returns before its artifacts resolve), so the
        // step-wide totals are exactly the landed tasks'
        // output. The union deduplicates the rest.
        let produced = tally.produced();
        match self.sequence_resume.sequence_checkpoint_record(
            &self.f_id,
            step_id,
            &landed_ids,
            anchor,
            Some(&produced),
            crate::paths::now_ms(),
        ) {
            Ok(total) => Some(total as usize),
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_id,
                    error = %e,
                    "failed to persist sequence checkpoint"
                );
                Some(landed_ids.len())
            }
        }
    }

    /// Tear the step's worktree down and reset the feature branch to
    /// `base_sha`, so a retry starts from the tip the step began at.
    ///
    /// **The order is load-bearing.** `merge_subtask` checks the feature
    /// branch *out inside this worktree* when it is not checked out anywhere
    /// else (which is the normal case — the feature branch is otherwise just a
    /// ref). Git refuses to `branch -f` a branch that a worktree holds, so
    /// rolling back before removing the worktree fails with "cannot force
    /// update the branch ... used by worktree at ...". Remove first, then
    /// reset.
    ///
    /// Returns whether the branch actually moved back. Callers must not claim
    /// a rollback they did not get: if the worktree could not be removed (a
    /// locked file — the case `provision_subtask_worktree` explicitly warns
    /// about), the reset fails and the step's commits are still on the branch.
    ///
    /// **The checkpoint rolls back with the branch**, per `checkpoint`. That
    /// is not tidying: since V35 the row does not merely say *skip these
    /// ids*, it names a commit the next attempt will `reset --hard` onto. A
    /// rollback that moved the branch back and left the row alone would hand
    /// the retry the very commits it just discarded — a verifier's rejection
    /// reinstated, an explicit cancel undone.
    ///
    /// [`CheckpointDisposition::RewindTo`] is the answer rather than a clear,
    /// because only *this attempt's* claim is being dropped: an earlier
    /// attempt's merged prefix is on the feature branch, `base_sha` was
    /// captured after it, and re-running it would be re-paying for work this
    /// rollback never touched. See [`ExecutionDriver::rewind_checkpoint_to`].
    ///
    /// **The checkpoint moves only if the branch did.** The reset is what can
    /// fail, so it goes first: rewinding a row to "these commits are gone"
    /// while `branch -f` left them on the branch describes a rollback that
    /// did not happen, and the next attempt would re-run tasks whose commits
    /// are still sitting there. A failed reset therefore leaves the row
    /// exactly as this attempt grew it — consistent with the branch, which is
    /// the state the caller reports and the user acts on.
    ///
    /// `cleanup_sequence_worktree` does delete the step branch; whatever the
    /// rewound checkpoint still pins stays reachable through its ref, so the
    /// next attempt can restore it.
    pub(crate) async fn cleanup_and_rollback(
        &self,
        step: StepCtx<'_>,
        target: RunTarget<'_>,
        wt_id: &str,
        base_sha: &Sha,
        checkpoint: CheckpointDisposition<'_>,
    ) -> bool {
        let machine_str = target.machine;
        let step_id = step.step_id();

        self.cleanup_sequence_worktree(wt_id).await;

        let reset = self
            .sequence_git(machine_str)
            .branch_force(&self.target_dir, &self.branch_name, base_sha)
            .await;

        if reset.is_ok() {
            match checkpoint {
                CheckpointDisposition::RewindTo(resume) => {
                    self.rewind_checkpoint_to(step_id, machine_str, resume)
                        .await
                }
                CheckpointDisposition::Discard => {
                    tracing::info!(
                        feature_id = %self.f_id,
                        step_id = %step_id,
                        "sequence step: discarding the landed checkpoint — its work is what \
                         this attempt's verdict rejected, so the retry re-implements it"
                    );
                    self.clear_sequence_checkpoint(step_id, machine_str).await;
                }
                CheckpointDisposition::Keep => {}
            }
        } else if !matches!(checkpoint, CheckpointDisposition::Keep) {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_id,
                "sequence step: the branch reset failed, so the landed checkpoint was left \
                 as this attempt grew it — it still matches what is on the branch"
            );
        }

        match reset {
            Ok(_) => true,
            Err(e) => {
                tracing::error!(
                    feature_id = %self.f_id,
                    branch = %self.branch_name,
                    base_sha = %base_sha,
                    error = %e,
                    "sequence step: could not roll the feature branch back; the failed step's \
                     commits are still on it"
                );
                false
            }
        }
    }

    /// Translate a rejected verifier into the step's outcome.
    ///
    /// A failed rollback leaves the rejected attempt's commits on
    /// the feature branch. Fold the warning into the verdict so it
    /// reaches both the stored error and the retry feedback —
    /// `resolve_task_plan` treats a step's own failure as rolled
    /// back and would otherwise tell the next attempt's agents the
    /// branch is clean. Only the failure case is decorated: a
    /// clean rollback is the normal path and needs no note in the
    /// verdict the retry prompts render.
    pub(crate) fn verifier_failure_outcome(
        &self,
        verifier_err: crate::domain::verifier::VerifierError,
        rolled_back: bool,
    ) -> StepOutcome {
        let decorate = |m: &str| -> String {
            if rolled_back {
                m.to_string()
            } else {
                FailureDisposition::RollbackFailed.decorate(m, &self.branch_name)
            }
        };
        match verifier_err {
            crate::domain::verifier::VerifierError::Verdict(mut failure) => {
                failure.reason = decorate(&failure.reason);
                StepOutcome::VerdictFailed(failure)
            }
            crate::domain::verifier::VerifierError::Infrastructure(msg) => {
                StepOutcome::NonRetryable(decorate(&format!(
                    "[verifier infrastructure error — check verifier config] {}",
                    msg
                )))
            }
            // Triaged as an environment problem: the box is not
            // provisioned, editing source cannot fix it.
            crate::domain::verifier::VerifierError::Environment(msg) => {
                StepOutcome::NonRetryable(decorate(&msg))
            }
            // Stop was pressed while the harness was running. Undecorated on
            // purpose: `decorate` explains a *failed* step's rollback state,
            // and a cancel is not a failure.
            crate::domain::verifier::VerifierError::Cancelled => StepOutcome::Cancelled,
        }
    }

    /// Persist the failure and translate it into the right outcome.
    ///
    /// Cancellation wins over whatever the error says, and that is not
    /// belt-and-braces. A cancel that lands while an agent turn is in
    /// flight comes back as [`SequenceError::Cancelled`] — but one that
    /// lands a moment later, while the task is committing or the step is
    /// merging, surfaces as an ordinary `Failed` from a git command whose
    /// worktree is being torn down underneath it. Only the watch can tell
    /// that apart from a real failure, so it is consulted first and the
    /// error variant decides the rest.
    pub(crate) async fn fail_sequence_step(
        &self,
        step: StepCtx<'_>,
        spend: &StepSpend<'_>,
        err: SequenceError,
        disposition: FailureDisposition,
    ) -> StepOutcome {
        let step_exec = step.step_exec;
        let (cost, tokens) = (*spend.cost, *spend.tokens);

        let is_cancelled = matches!(err, SequenceError::Cancelled) || *self.cancel_watch.borrow();
        let status_str = if is_cancelled {
            "interrupted"
        } else {
            "failed"
        };
        let wall = spend.start.elapsed().as_secs();
        let stored = disposition.decorate(&err.to_string(), &self.branch_name);
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some(status_str.to_string()),
                cost_usd: Some(Some(cost)),
                tokens: Some(Some(tokens)),
                wall_clock_secs: Some(Some(wall)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(Some(stored)),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: status_str.into(),
            cost_usd: Some(cost),
            tokens: Some(tokens),
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });

        if is_cancelled {
            return StepOutcome::Cancelled;
        }
        err.into()
    }
}
