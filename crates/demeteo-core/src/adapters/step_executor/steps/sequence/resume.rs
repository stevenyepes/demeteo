//! The crash-resume checkpoint: everything that reads, pins, moves or spends
//! the record of what a previous attempt already landed.
//!
//! All of it is *mechanism*. What a checkpoint means — whether its prefix can
//! be restored, whether a verdict should keep or discard it — is decided in
//! [`crate::domain::sequence::checkpoint`], synchronously and without a port
//! in sight. This module only asks git the questions that decision needs, and
//! performs whatever it returns.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::models::{CheckpointProduced, StepExecution};
use crate::domain::sequence::checkpoint::{self, CheckpointResume};

use super::git::SequenceGit;

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/probe.rs"]
mod probe_tests;

/// Which run a [`probe_anchor`] warning belongs to. Only ever read by
/// `tracing`: the probe itself needs nothing from the driver, which is what
/// lets a test drive it with a scripted `ExecutionPort` and no driver at all.
pub(crate) struct ProbeLog<'a> {
    pub feature_id: &'a str,
    pub step_id: &'a str,
}

/// Ask git where the checkpoint's anchor commit stands relative to the
/// feature branch.
///
/// Two questions and no decisions — the verdict is
/// [`AnchorProbe`](crate::domain::sequence::checkpoint::AnchorProbe), and
/// what it *means* is
/// [`classify`](crate::domain::sequence::checkpoint::classify)'s to say.
///
/// A free function rather than a method because it needs an
/// `ExecutionPort` and four strings, not a run: `ExecutionDriver` carries
/// twenty-odd ports that a test would have to stub to reach a method here,
/// which is the cost that left this logic uncovered.
pub(crate) async fn probe_anchor(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_str: &str,
    target_dir: &str,
    anchor: &str,
    base_sha: &str,
    log: ProbeLog<'_>,
) -> checkpoint::AnchorProbe {
    let git = SequenceGit::new(exec, machine_str);

    // Is the prefix still there to restore? The companion ref keeps it
    // reachable, so a miss means the ref was deleted or the repo was
    // replaced under us — either way there is nothing to resume onto.
    if let Err(e) = git.commit_exists(target_dir, anchor).await {
        tracing::warn!(
            feature_id = %log.feature_id,
            step_id = %log.step_id,
            anchor = %anchor,
            error = %e,
            "sequence step: the landed checkpoint's anchor commit is unreachable; \
             running the full task list"
        );
        return checkpoint::AnchorProbe::Missing;
    }

    match git.merge_base(target_dir, anchor, base_sha).await {
        Ok(out) if checkpoint::anchor_is_merged(&out, anchor) => checkpoint::AnchorProbe::Merged,
        Ok(_) => checkpoint::AnchorProbe::Stranded,
        Err(e) => {
            tracing::warn!(
                feature_id = %log.feature_id,
                step_id = %log.step_id,
                error = %e,
                "sequence step: could not tell whether the landed prefix is merged; \
                 running the full task list"
            );
            checkpoint::AnchorProbe::Unknown
        }
    }
}

impl ExecutionDriver {
    /// Decide what a previous attempt's landed prefix means for this one.
    ///
    /// Runs against the main repo, before any worktree exists, because
    /// `resolve_task_plan` needs the answer: the ids it drops and the
    /// commits this restores have to be the same set.
    ///
    /// All this does is *observe* — read the row, ask git the two
    /// questions — and hand both to
    /// [`checkpoint::classify`](crate::domain::sequence::checkpoint::classify),
    /// which owns the decision. A read that fails is
    /// [`AnchorProbe::Unknown`](checkpoint::AnchorProbe::Unknown) like any
    /// other unanswered question, and classify resolves every uncertainty to
    /// [`CheckpointResume::None`]: a full re-run wastes money, while a
    /// wrong skip loses work.
    pub(crate) async fn resolve_checkpoint_resume(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        base_sha: &str,
    ) -> CheckpointResume {
        let checkpoint = match self
            .sequence_resume
            .sequence_checkpoint_get(&self.f_id, &step_exec.step_id.0)
        {
            Ok(cp) => cp,
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    error = %e,
                    "sequence step: could not read the landed checkpoint; running the \
                     full task list"
                );
                return CheckpointResume::None;
            }
        };

        // Only a row that carries an anchor has anything to probe; an
        // anchor-less one is pre-V35 and classify reads it without asking.
        let probe = match checkpoint.anchor_sha.as_deref() {
            Some(anchor) if !checkpoint.is_empty() => {
                probe_anchor(
                    &*self.exec,
                    machine_str,
                    &self.target_dir,
                    anchor,
                    base_sha,
                    ProbeLog {
                        feature_id: self.f_id.as_str(),
                        step_id: &step_exec.step_id.0,
                    },
                )
                .await
            }
            _ => checkpoint::AnchorProbe::Unknown,
        };

        checkpoint::classify(checkpoint, probe)
    }

    /// The shared git ref pinning this step's landed prefix.
    ///
    /// The prefix lives on the step branch until the step completes, and
    /// `provision_subtask_worktree`'s leftover-state path resets that branch
    /// back to the feature branch — which would orphan every commit an
    /// interrupted attempt made, leaving them one `git gc` from
    /// unrecoverable. A ref outside `refs/heads` keeps them reachable
    /// without adding a branch to the user's `git branch` output. It is a
    /// *shared* ref (git's per-worktree namespaces are `HEAD`,
    /// `refs/bisect/*`, `refs/worktree/*` and `refs/rewritten/*`), so the
    /// step worktree and the main checkout resolve the same one, and it
    /// outlives the worktree that wrote it.
    fn checkpoint_ref(&self, step_id: &str) -> String {
        format!("refs/demeteo/seq/{}/{}", self.f_id_str, step_id)
    }

    /// Record one task as landed, durably, the moment its commit exists.
    ///
    /// This is the difference between a crash costing one task and costing
    /// the whole list. The mid-list failure path also checkpoints, but
    /// only when `run_tasks_loop` *returns* — a killed process never gets
    /// there, so before this existed the ids of twenty finished tasks died
    /// with the driver and the next attempt re-planned from task one.
    ///
    /// Both writes are best-effort, and the order between them is not: the
    /// row names a commit a later attempt will `reset --hard` to, so the
    /// commit is pinned *first*. A failed pin means the task goes
    /// unrecorded and re-runs — wasteful, but not wrong. A row naming an
    /// unpinned commit would be worse than either.
    ///
    /// `produced` is what *this* task emitted (V36), recorded on the same
    /// write as its id so the two can never disagree about which tasks the
    /// payload covers. An attempt that resumes a fully-landed list runs no
    /// task, and this row is then the only surviving record of what the
    /// step has to show for itself.
    pub(crate) async fn checkpoint_landed_task(
        &self,
        step_exec: &StepExecution,
        machine_str: &str,
        task_id: &str,
        sha: &str,
        produced: &CheckpointProduced,
    ) {
        let git_ref = self.checkpoint_ref(&step_exec.step_id.0);
        if let Err(e) = self
            .sequence_git(machine_str)
            .update_ref(&self.target_dir, &git_ref, sha)
            .await
        {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                task_id = %task_id,
                error = %e,
                "sequence task: could not pin the landed prefix; it will not be \
                 checkpointed and will re-run if this attempt is interrupted"
            );
            return;
        }

        if let Err(e) = self.sequence_resume.sequence_checkpoint_record(
            &self.f_id,
            &step_exec.step_id.0,
            &[task_id.to_string()],
            Some(sha),
            Some(produced),
            crate::paths::now_ms(),
        ) {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                task_id = %task_id,
                error = %e,
                "sequence task: could not persist the landed checkpoint"
            );
        }
    }

    /// Spend the checkpoint: drop the row and unpin the prefix.
    ///
    /// Called when the prefix stops being the thing a resume should restore
    /// because the step *completed*. The row goes first: a leftover pinned
    /// commit is inert, while a surviving row pointing at an unpinned commit
    /// is a resume that can fail its `cat-file` probe for no reason.
    pub(crate) async fn clear_sequence_checkpoint(&self, step_id: &str, machine_str: &str) {
        let _ = self
            .sequence_resume
            .sequence_checkpoint_clear(&self.f_id, step_id);
        self.unpin_checkpoint_prefix(step_id, machine_str).await;
    }

    /// Point the checkpoint ref at `sha`, or delete it when `sha` is `None`.
    async fn move_checkpoint_ref(&self, step_id: &str, machine_str: &str, sha: Option<&str>) {
        let git_ref = self.checkpoint_ref(step_id);
        let git = self.sequence_git(machine_str);
        let _ = match sha {
            Some(sha) => git.update_ref(&self.target_dir, &git_ref, sha).await,
            None => git.delete_ref(&self.target_dir, &git_ref).await,
        };
    }

    /// Unpin the prefix entirely.
    async fn unpin_checkpoint_prefix(&self, step_id: &str, machine_str: &str) {
        self.move_checkpoint_ref(step_id, machine_str, None).await;
    }

    /// Put the checkpoint back to the row this attempt started from.
    ///
    /// The counterpart to `cleanup_and_rollback`'s branch reset, and the
    /// reason the two are called together: a rollback that moved the feature
    /// branch back while leaving the checkpoint pointing at this attempt's
    /// commits would not be a rollback at all. The next attempt reads that
    /// row, finds an anchor that is not an ancestor of the (rewound) branch,
    /// and `reset --hard`s the fresh worktree onto exactly the commits the
    /// rollback set out to discard — so a verifier's rejection, or a cancel,
    /// would quietly reinstate the work it rejected.
    ///
    /// Rewinding to [`CheckpointResume`] rather than clearing outright is
    /// what keeps an *earlier* attempt's merged prefix: that work is on the
    /// feature branch, `base_sha` was captured after it, and this attempt
    /// never had any claim on it.
    ///
    /// The ref moves back too. The task loop advanced it once per landed
    /// task, so leaving it at the tip would keep this attempt's discarded
    /// commits reachable forever — and, worse, would survive a later
    /// `Merged` rewind that expects no pin at all.
    pub(crate) async fn rewind_checkpoint_to(
        &self,
        step_id: &str,
        machine_str: &str,
        resume: &CheckpointResume,
    ) {
        let (landed_ids, anchor, produced) = resume.as_stored();
        if landed_ids.is_empty() {
            self.clear_sequence_checkpoint(step_id, machine_str).await;
        } else {
            if let Err(e) = self.sequence_resume.sequence_checkpoint_set(
                &self.f_id,
                step_id,
                landed_ids,
                anchor,
                produced,
                crate::paths::now_ms(),
            ) {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_id,
                    error = %e,
                    "sequence step: could not rewind the landed checkpoint after a rollback; \
                     the next attempt may restore commits this one discarded"
                );
            }
            self.move_checkpoint_ref(step_id, machine_str, anchor).await;
        }
        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_id,
            landed = landed_ids.len(),
            anchor = anchor.unwrap_or("-"),
            "sequence step: rewound the landed checkpoint with the branch"
        );
    }
}
