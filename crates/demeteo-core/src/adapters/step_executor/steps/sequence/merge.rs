//! Getting the step's commits onto the feature branch — the one merge on the
//! success path, and the partial one a mid-list failure salvages.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::spend::RunningSpend;
use crate::adapters::step_executor::steps::conflict_pass::{ConflictPass, ConflictPassError};
use crate::domain::sequence::outcome::SequenceError;
use crate::domain::sequence::progress::LandedTask;

use super::context::{RunTarget, StepCtx, StepSpend, StepWorktree};

impl ExecutionDriver {
    /// Merge the step's task branch into the feature branch, spending one
    /// agent turn on conflict resolution if the merge conflicts.
    ///
    /// Unlike the agent step, there is no live session to reuse here — each
    /// task's session is killed as soon as its task commits, deliberately, so
    /// no context carries across tasks. So the resolution pass gets its own
    /// fresh session in the step's worktree. It only ever sees the conflicted
    /// files, which is all it needs.
    ///
    /// `Err` when the merge could not be made to land; the caller rolls
    /// the feature branch back.
    pub(crate) async fn merge_with_conflict_recovery(
        &self,
        step: StepCtx<'_>,
        spend: &mut StepSpend<'_>,
        target: RunTarget<'_>,
        wt: StepWorktree<'_>,
    ) -> Result<(), SequenceError> {
        let merge_err = match self
            .git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                wt.path,
                &self.branch_name,
                wt.id,
            )
            .await
        {
            Ok(()) => return Ok(()),
            Err(e) => e,
        };

        let conflict_thread_id = format!("{}-{}-merge", self.f_id_str, step.step_id());
        // Resolving Demeteo's own merge, not the step's work: the step's
        // answer does not reach this turn however the workflow set it.
        let target = RunTarget {
            keep_harness_personalization: crate::domain::turn_role::TurnRole::Orchestrator
                .keeps_harness_personalization(),
            ..target
        };
        let session = self
            .spawn_sequence_session(
                target,
                wt.path,
                &conflict_thread_id,
                "resolve merge conflicts",
            )
            .await?;

        let pass = self
            .resolve_merge_conflicts_via_agent(
                step.step_exec,
                &*session,
                target.machine,
                wt.path,
                target.override_model,
                RunningSpend {
                    cost: spend.cost,
                    tokens: spend.tokens,
                    start: spend.start,
                },
            )
            .await;
        let _ = self.registry.kill(&conflict_thread_id).await;

        match pass {
            // Not a content conflict — an agent cannot help. Report the
            // original merge error, which says what actually went wrong.
            Ok(ConflictPass::NothingToResolve) => Err(SequenceError::Failed(format!(
                "sequence step: merging the completed task branch into '{}' failed: {}",
                self.branch_name, merge_err
            ))),
            Ok(ConflictPass::Resolved(_)) => self
                .git_ops
                .merge_subtask(
                    self.machine_id_opt.as_deref(),
                    wt.path,
                    &self.branch_name,
                    wt.id,
                )
                .await
                .map_err(|e| {
                    SequenceError::Failed(format!(
                        "sequence step: merging into '{}' still failed after the agent \
                         resolved the conflicts: {}",
                        self.branch_name, e
                    ))
                }),
            Err(ConflictPassError::Cancelled) => Err(SequenceError::Cancelled),
            Err(ConflictPassError::Failed(msg)) => Err(SequenceError::Failed(format!(
                "sequence step: could not resolve the conflicts merging into '{}': {}",
                self.branch_name, msg
            ))),
            Err(ConflictPassError::Environmental(msg)) => {
                Err(SequenceError::Environmental(format!(
                    "sequence step: agent error while resolving the merge conflicts: {}",
                    msg
                )))
            }
        }
    }

    /// Preserve the completed task prefix after a mid-list failure: reset the
    /// worktree to the last completed task's commit — discarding the failed
    /// task's debris, both uncommitted writes and any commits its agent made
    /// itself — and merge the step's task branch into the feature branch.
    ///
    /// Only the *merge conflict* recovery is deliberately absent here. On the
    /// success path a conflicting merge is worth an agent turn, because the
    /// worktree holds a complete verified implementation. Here the step is
    /// already failing; spending more agent budget to salvage a partial
    /// prefix inverts that trade, so a conflict falls back to the ordinary
    /// full rollback in the caller.
    pub(crate) async fn checkpoint_landed_prefix(
        &self,
        machine_str: &str,
        wt: StepWorktree<'_>,
        landed: &[LandedTask],
    ) -> Result<(), String> {
        let last = landed
            .last()
            .ok_or_else(|| "no completed tasks to checkpoint".to_string())?;

        self.sequence_git(machine_str)
            .reset_hard(wt.path, &last.sha)
            .await
            .map_err(|e| {
                format!(
                    "could not reset the worktree to the last completed task's commit {}: {}",
                    last.sha, e
                )
            })?;

        self.git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                wt.path,
                &self.branch_name,
                wt.id,
            )
            .await
            .map_err(|e| {
                format!(
                    "merging the completed task prefix into '{}' failed: {}",
                    self.branch_name, e
                )
            })
    }
}
