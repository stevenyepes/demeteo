//! The single worktree a `sequence` step runs its whole task list in:
//! opening it, putting an interrupted attempt's work back into it, and
//! tearing it down again.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::sequence::checkpoint::CheckpointResume;
use crate::domain::sequence::tasks::TaskPlan;

use super::context::{RunTarget, StepCtx};

impl ExecutionDriver {
    /// One worktree for the whole step, feature-scoped exactly as an
    /// agent step's is. Two features on the same repo therefore get
    /// different directories, and nothing this step does can disturb a
    /// sibling feature's worktree.
    ///
    /// Returns the worktree path, ready for the task loop: the interrupted
    /// attempt's commits are back in the tree and the scope fence is up.
    pub(crate) async fn open_step_worktree(
        &self,
        step: StepCtx<'_>,
        target: RunTarget<'_>,
        wt_id: &str,
        plan: &TaskPlan,
        resume: &CheckpointResume,
    ) -> Result<String, StepOutcome> {
        let wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                wt_id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return Err(StepOutcome::Environmental(format!(
                    "sequence step: worktree provision failed ({}): {}",
                    wt_id, e
                )))
            }
        };

        // Put the interrupted attempt's work back. Provisioning cuts the
        // worktree from the feature branch — and its leftover-state path
        // explicitly resets the step branch there — so a prefix that only
        // ever lived on the step branch is *gone from the tree* by now,
        // though still reachable through the checkpoint ref. Move onto it
        // before the scope fence chmods anything read-only, since the
        // reset writes files.
        if let CheckpointResume::Restore {
            landed_ids, sha, ..
        } = resume
        {
            if let Err(e) = self
                .sequence_git(target.machine)
                .reset_hard(&wt_path, sha)
                .await
            {
                // The plan has already dropped these tasks, so running on
                // would implement the remainder against a tree missing
                // their work. Environmental: a retry re-probes, and if the
                // anchor is genuinely gone it resolves to a full re-run.
                self.cleanup_sequence_worktree(wt_id).await;
                return Err(StepOutcome::Environmental(format!(
                    "sequence step: could not restore the {} task(s) an interrupted \
                     attempt completed (checkpoint commit {}): {}",
                    landed_ids.len(),
                    sha,
                    e
                )));
            }
            tracing::info!(
                feature_id = %self.f_id,
                step_id = %step.step_id(),
                restored = landed_ids.len(),
                remaining = plan.tasks.len(),
                anchor = %sha,
                "sequence step: restored an interrupted attempt's committed work"
            );
        }

        // Scope fence. A no-op for `Implement` capability (whole worktree
        // writable), which is what a sequence step normally carries.
        if let Err(e) = self
            .git_ops
            .apply_artifact_scope(
                self.machine_id_opt.as_deref(),
                &wt_path,
                &self.sequence_writable_paths(step.step_conf),
            )
            .await
        {
            self.cleanup_sequence_worktree(wt_id).await;
            return Err(StepOutcome::Environmental(format!(
                "sequence step: artifact scope setup failed: {}",
                e
            )));
        }

        Ok(wt_path)
    }

    pub(crate) async fn cleanup_sequence_worktree(&self, wt_id: &str) {
        let _ = self
            .git_ops
            .cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                wt_id,
            )
            .await;
    }
}
