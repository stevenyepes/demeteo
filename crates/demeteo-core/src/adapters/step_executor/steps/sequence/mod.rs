//! The `sequence` step: run an ordered task list, one fresh agent per task,
//! all in a single worktree, and merge the result back once.
//!
//! This replaces the old `parallel` step, which fanned out one worker per
//! subtask across concurrent worktrees and merged each back independently.
//! That design cost more than it bought:
//!
//! * **Concurrent worktrees collide.** Worktree directories are derived from
//!   the repo dir, which every feature on a project shares. Provisioning
//!   force-removes its target, so one feature could delete a sibling's live
//!   worktree along with its uncommitted work.
//! * **N merges mean N chances to conflict.** Each subtask merged into the
//!   feature branch separately, so subtask 2 could conflict with subtask 1
//!   and needed an agent to resolve it mid-step.
//! * **Disjoint file ownership is a fiction.** The planner had to partition
//!   files up front so workers could not collide — but real work rarely
//!   partitions cleanly, and a task needing a file another task owned had no
//!   way to say so.
//!
//! Running the tasks in order in one worktree dissolves all three. There is
//! one worktree per (feature, step) — named with the same feature-scoped
//! convention agent steps use, so cross-feature collision is impossible.
//! There is one merge, at the end. And because each task commits before the
//! next begins, task N *sees* task N-1's work, so `files` need not be
//! disjoint and a later task can legitimately build on an earlier one.
//!
//! What survives from the old design is the part that actually helped: each
//! task gets a *fresh* agent session, so no single context window has to
//! carry the whole feature.
//!
//! # Where the step lives
//!
//! This file is the orchestration and nothing else — the stages in the order
//! they run, with the judgement between them. Each stage is a module:
//!
//! * [`plan`] / [`planner`] — where the ordered task list comes from
//! * [`resume`] — what a previous attempt already landed, and the checkpoint
//!   that records it
//! * [`worktree`] — the one worktree the whole list runs in
//! * [`runner`] / [`task`] / [`prompt`] / [`session`] — the tasks themselves
//! * [`merge`] — putting the commits on the feature branch
//! * [`rollback`] — unwinding an attempt that will not complete
//! * [`completion`] — what the step hands downstream
//! * [`git`] — every git command any of them issues
//!
//! The decisions those stages consult are not here either: they are
//! synchronous policy in [`crate::domain::sequence`], reachable from a test
//! without a port double.

use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::sequence::checkpoint::{
    verdict_disposition, CheckpointDisposition, CheckpointResume,
};
use crate::domain::sequence::outcome::{FailureDisposition, SequenceError};
use crate::domain::sequence::progress::StepTally;
use crate::ports::notification::DomainEvent;

mod completion;
mod git;
mod handler;
mod merge;
mod plan;
mod planner;
mod prompt;
mod resume;
mod rollback;
mod runner;
mod schema;
mod session;
mod task;
mod worktree;

pub(crate) use handler::SequenceNodeHandler;

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/steps/sequence/disposition.rs"]
mod disposition_tests;

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_sequence_step(
        &mut self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        if *self.cancel_watch.borrow() {
            return StepOutcome::Cancelled;
        }

        // A fresh attempt cannot have live task rows of its own, so any
        // `running` subtask_runs row for this step is a leftover from a
        // crashed or killed process. The startup watchdog closes these for
        // features it reconciles, but resume paths it skips (runner-owned
        // features, a driver re-running the step) land here — close them, or
        // the dashboard's "nodes" count over-reports forever.
        if let Err(e) = self
            .subtask_runs
            .subtask_runs_interrupt_stale(&step_exec.id, crate::paths::now_ms())
        {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                error = %e,
                "sequence step: could not close stale subtask_runs rows"
            );
        }

        let (agent_kind, override_model) = self.resolve_step_agent(step_conf);
        let machine_str = self.machine_id().to_string();

        // Rollback anchor: the feature branch tip before this attempt. On
        // failure we reset the branch ref back to it so a retry starts clean.
        let base_sha = match self
            .sequence_git(&machine_str)
            .rev_parse(&self.target_dir, &self.branch_name)
            .await
        {
            Ok(s) => s.trim().to_string(),
            Err(e) => {
                return StepOutcome::Failed(format!(
                    "sequence step: could not capture base SHA for rollback anchor: {}",
                    e
                ))
            }
        };

        // 0. What did a previous attempt already finish? Resolved before the
        //    plan because it decides which tasks the plan may drop, and
        //    before the worktree because it is a question about the repo.
        let resume = self
            .resolve_checkpoint_resume(step_exec, &machine_str, &base_sha)
            .await;

        // 1. Resolve the ordered task list.
        let retry_iteration = self.retry_ctx.as_ref().map(|rc| rc.iteration).unwrap_or(0);
        let plan = match self
            .resolve_task_plan(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                retry_iteration,
                &agent_kind,
                override_model.as_deref(),
                &machine_str,
                step_execs,
                step_index,
                &resume,
            )
            .await
        {
            Ok(p) => p,
            Err(outcome) => return outcome,
        };

        // Nothing to run splits two ways, and only one of them is a failure.
        // A plan with no tasks *and* nothing landed is a misconfigured step.
        // A plan whose every task is already checkpointed is a step resuming
        // into its own tail — killed between the last task's commit and the
        // merge — and it still has the artifact check, the verifier and that
        // merge to do. Failing it here would re-run the whole list on the
        // next attempt, which is the cost this checkpoint exists to avoid.
        let resumed_whole_list = plan.tasks.is_empty() && !plan.already_landed.is_empty();
        if plan.tasks.is_empty() && plan.already_landed.is_empty() {
            return StepOutcome::Failed(
                "sequence step: the task list is empty — there is nothing to implement."
                    .to_string(),
            );
        }
        if resumed_whole_list {
            tracing::info!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                landed = plan.already_landed.len(),
                "sequence step: every task already landed; resuming straight to verify and merge"
            );
        }

        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            tasks = plan.tasks.len(),
            attempt = retry_iteration,
            "sequence step: running tasks in order"
        );

        // 2. One worktree for the whole step, carrying whatever an
        //    interrupted attempt had already committed.
        let wt_id = format!("{}-step-{}", self.f_id_str, step_exec.step_id.0);
        let wt_path = match self
            .open_step_worktree(step_exec, step_conf, &machine_str, &wt_id, &plan, &resume)
            .await
        {
            Ok(p) => p,
            Err(outcome) => return outcome,
        };

        // 3. Run the tasks in order.
        //
        // The tally does not start empty on a resume. Its totals are
        // step-wide judgements — what this step hands downstream, and which
        // declared deliverables exist — and a task that landed under an
        // earlier attempt contributed to both. Its contribution lived only
        // in this process's memory until V36; a killed attempt took it with
        // it, and the next one would either starve its consumers of the
        // refs or fail the step for a deliverable already on disk. The
        // checkpoint now carries it, so the resume reads it back.
        //
        // `None` from `produced()` is a pre-V36 row and means *unknown* —
        // handled at 3b, not here, because the two halves degrade
        // differently.
        let mut tally = StepTally::resuming(resume.produced());
        let tasks_res = self
            .run_tasks_loop(
                step_exec,
                step_conf,
                accumulated_cost,
                accumulated_tokens,
                step_start,
                step_index,
                step_execs,
                &plan,
                &machine_str,
                &wt_id,
                &wt_path,
                &agent_kind,
                override_model.as_deref(),
                &mut tally,
            )
            .await;

        if let Err(task_err) = tasks_res {
            // A mid-list failure does not forfeit the tasks that already
            // finished: their work is committed in the worktree and paid
            // for. Merge that prefix to the feature branch and record it, so
            // the retry runs only the remainder (decision 13's
            // continue-and-report intent, adapted to ordered tasks — the
            // tail may depend on the failed task, so it is not run, but the
            // completed prefix is kept). Cancellation is not a failure:
            // the user asked to stop, so the branch rolls back as before —
            // and, since V35, so does the checkpoint the task loop grew on
            // the way here. Without that rewind, "stop" would leave a resume
            // point that restores the stopped attempt's commits, which is
            // the opposite of what the rollback below is for.
            let mut checkpoint = CheckpointDisposition::RewindTo(&resume);
            if !*self.cancel_watch.borrow() && !tally.landed().is_empty() {
                match self
                    .salvage_landed_prefix(step_exec, &wt_id, &wt_path, &machine_str, &tally)
                    .await
                {
                    Some(landed_total) => {
                        self.cleanup_sequence_worktree(&wt_id).await;
                        return self
                            .fail_sequence_step(
                                step_exec,
                                step_start,
                                *accumulated_cost,
                                *accumulated_tokens,
                                task_err,
                                FailureDisposition::PrefixLanded {
                                    landed: landed_total,
                                    // Everything the checkpoint knows landed,
                                    // plus this attempt's plan minus what it
                                    // ran. The tally only ever lands tasks
                                    // drawn from `plan.tasks`, so the
                                    // difference cannot go negative — but the
                                    // invariant lives a module away in
                                    // `run_tasks_loop`, and a count in a
                                    // user-facing message is not worth a
                                    // panic if it ever moves.
                                    total: landed_total
                                        + plan.tasks.len().saturating_sub(tally.landed().len()),
                                },
                            )
                            .await;
                    }
                    // The merge failed, so the prefix is still on the step
                    // branch — but it is finished, paid-for work, and the
                    // task loop already pinned and recorded it. Keep that
                    // claim: the next attempt restores those commits and
                    // runs only the remainder, which is strictly better
                    // than re-running them into the same conflict. This is
                    // the one rollback that is about the feature branch
                    // rather than about disowning the attempt.
                    None => checkpoint = CheckpointDisposition::Keep,
                }
            }
            let rolled_back = self
                .cleanup_and_rollback(
                    &wt_id,
                    &machine_str,
                    &base_sha,
                    &step_exec.step_id.0,
                    checkpoint,
                )
                .await;
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    task_err,
                    FailureDisposition::from_rollback(rolled_back),
                )
                .await;
        }

        // 3b. Declared deliverables must exist. A `ByName` / `LastWriteTo`
        //     artifact that no task ever produced means the step ran and wrote
        //     nothing downstream can consume — the same misconfiguration class
        //     the agent step fails on (wrong model, blocked writes, agent wrote
        //     to the wrong path). Judged here, across the whole task list,
        //     rather than per task: only one task in the list may be the one
        //     that writes the report, so a per-task check would fail every
        //     other task spuriously.
        //
        //     `AllWrites` / `ChangedFiles` / `Diff` captures can never be
        //     missing (they describe whatever the agent touched), so an
        //     ordinary implement step never trips this.
        //
        //     Both halves of that judgement assume a task ran this attempt.
        //     When every task was already landed, none did — so both read
        //     from the checkpoint's produced payload, seeded into the
        //     accumulators at step 3, and the check runs exactly as it does
        //     for an attempt that ran the list. That is the point of
        //     persisting it: a resumed step is judged on the same evidence,
        //     not exempted from the judgement.
        //
        //     A pre-V36 row cannot answer, and `None` there means *unknown*,
        //     never *empty*. Only that case keeps the old fallback: sweep
        //     whatever the store holds for the step so the refs reach
        //     downstream, and skip a check whose input is missing rather
        //     than fail a step whose deliverable is sitting on disk. The
        //     sweep is not equivalent to the payload — the store has no
        //     attempt dimension, so it also names files an earlier,
        //     rolled-back attempt wrote — which is why it is the
        //     compatibility path and not the resume path.
        let produced_unknown = resume.produced().is_none();
        let unjudgeable = resumed_whole_list && produced_unknown;
        if unjudgeable {
            match self
                .artifacts
                .list_for_step(&self.f_id_str, &step_exec.step_id.0)
            {
                Ok(stored) => tally.recover_refs(stored),
                Err(e) => tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    error = %e,
                    "sequence step: could not recover the landed tasks' artifacts; the step \
                     will complete carrying only its diff"
                ),
            }
        }
        let never_produced: Vec<&str> = step_conf
            .artifacts
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .filter(|d| !tally.satisfies(&d.name))
            .map(|d| d.name.as_str())
            .collect();
        if !unjudgeable && !never_produced.is_empty() {
            let rolled_back = self
                .cleanup_and_rollback(
                    &wt_id,
                    &machine_str,
                    &base_sha,
                    &step_exec.step_id.0,
                    verdict_disposition(resumed_whole_list, &resume),
                )
                .await;
            let names = never_produced.join(", ");
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    SequenceError::Failed(format!(
                        "sequence step: every task ran, but the declared artifact(s) {} were \
                         never produced by any task. Nothing downstream can consume this step — \
                         the agent may have written to a different path, or been blocked by its \
                         model/config.",
                        names
                    )),
                    FailureDisposition::from_rollback(rolled_back),
                )
                .await;
        }

        // 4. Verifier (harness / judge), if the step declares one. It runs
        //    against the worktree — where every task's commits already are —
        //    so it sees the complete change before anything reaches the
        //    feature branch.
        if let Some(ref verifier_cfg) = step_conf.verifier {
            let verifier_result = self
                .run_verifier_logic(
                    step_exec,
                    verifier_cfg,
                    &wt_path,
                    &[],
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                    &agent_kind,
                    override_model.as_deref(),
                    &machine_str,
                )
                .await;

            if let Err(verifier_err) = verifier_result {
                let _ = self
                    .registry
                    .kill(&format!("{}-verifier", self.f_id.as_str()))
                    .await;
                let rolled_back = self
                    .cleanup_and_rollback(
                        &wt_id,
                        &machine_str,
                        &base_sha,
                        &step_exec.step_id.0,
                        verdict_disposition(resumed_whole_list, &resume),
                    )
                    .await;
                return self.verifier_failure_outcome(verifier_err, rolled_back);
            }
        }

        // 5. One merge, at the end. The tasks cannot have conflicted with
        //    each other (same worktree, sequential, each committed), so the
        //    only way this conflicts is the feature branch having moved
        //    beneath us — e.g. a `sync` step pulled upstream in a prior step.
        //
        //    That is worth recovering from rather than failing on: the
        //    worktree holds a complete, verified implementation of every
        //    task, and throwing it away over conflict markers would mean
        //    re-running the whole task list. So we spend one agent turn
        //    resolving the conflict and retry the merge.
        let merge_res = self
            .merge_with_conflict_recovery(
                step_exec,
                &wt_id,
                &wt_path,
                &machine_str,
                &agent_kind,
                override_model.as_deref(),
                self.resolve_step_effort(step_conf),
                accumulated_cost,
                accumulated_tokens,
                step_start,
            )
            .await;

        if let Err(merge_err) = merge_res {
            let _ = self.notif.emit(&DomainEvent::ConflictDetected {
                feature_id: self.f_id.clone(),
                subtask_id: crate::adapters::worktree::git_ops::subtask_branch_name(
                    &self.branch_name,
                    &wt_id,
                ),
            });
            let rolled_back = self
                .cleanup_and_rollback(
                    &wt_id,
                    &machine_str,
                    &base_sha,
                    &step_exec.step_id.0,
                    CheckpointDisposition::RewindTo(&resume),
                )
                .await;
            return self
                .fail_sequence_step(
                    step_exec,
                    step_start,
                    *accumulated_cost,
                    *accumulated_tokens,
                    merge_err,
                    FailureDisposition::from_rollback(rolled_back),
                )
                .await;
        }

        // 6. What the step hands downstream: the feature's diff plus every
        //    task's artifacts.
        let mut refs = self
            .collect_step_refs(step_exec, &machine_str, &base_sha, &tally)
            .await;

        // Every task ran and committed, yet the branch carries nothing.
        //
        // On a retry that is legitimate: a targeted re-run whose fix was
        // already in place is a no-op, and there are prior-attempt artifacts
        // worth keeping in scope for the critic. On the first attempt it
        // means the implementation produced nothing at all — and reporting
        // `completed` there is exactly what used to hand a green status to
        // `s-validate`, which then correctly found the feature unimplemented.
        // Fail instead.
        if refs.is_empty() {
            if retry_iteration == 0 {
                // The merge has already landed by this point, so this path has
                // to undo it — otherwise a retry starts from a branch carrying
                // an empty merge commit.
                let rolled_back = self
                    .cleanup_and_rollback(
                        &wt_id,
                        &machine_str,
                        &base_sha,
                        &step_exec.step_id.0,
                        verdict_disposition(resumed_whole_list, &resume),
                    )
                    .await;
                return self
                    .fail_sequence_step(
                        step_exec,
                        step_start,
                        *accumulated_cost,
                        *accumulated_tokens,
                        SequenceError::Failed(
                            "sequence step: every task completed but the feature branch \
                             carries no changes — the implementation produced nothing."
                                .to_string(),
                        ),
                        FailureDisposition::from_rollback(rolled_back),
                    )
                    .await;
            }
            refs = step_exec.artifact_paths.clone();
        }

        self.cleanup_sequence_worktree(&wt_id).await;

        // The step is done, so any mid-list checkpoint is spent: from here on
        // the ordinary "previous attempt landed" retry logic covers the
        // branch's contents, and a stale skip-list would silently exempt
        // tasks from a future full re-run. Unpinning the prefix here is also
        // what eventually collects a ref left behind by a replay, which
        // drops the row without a repo context to delete the ref from.
        self.clear_sequence_checkpoint(&step_exec.step_id.0, &machine_str)
            .await;

        self.mark_step_completed(
            step_exec,
            step_start,
            *accumulated_cost,
            *accumulated_tokens,
            refs,
        )
    }
}
