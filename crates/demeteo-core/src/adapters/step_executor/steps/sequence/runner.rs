//! The ordered task loop: every task in the list, one after another, in the
//! one worktree — plus the per-task bookkeeping (telemetry row, checkpoint,
//! tally) that has to happen whether the next task runs or not.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::sequence::outcome::SequenceError;
use crate::domain::sequence::progress::{LandedTask, StepTally};
use crate::domain::sequence::tasks::TaskPlan;

use super::context::{RunTarget, StepCtx, StepSpend, StepWorktree, TaskRun};
use super::prompt::CompletedTask;

/// Aggregate dollar ceiling for one `sequence` step's whole task list, as a
/// multiple of the resolved per-task budget.
///
/// Each task's own turn is capped by `role_max_budget_usd` (see
/// `run_one_task`), but that ceiling resets for *every* task — nothing else
/// bounds how much a long, or over-decomposed, task list spends in total
/// ([`validate_task_plan`](crate::domain::sequence::tasks::validate_task_plan)
/// deliberately places no cap on task count). This is the actual backstop:
/// generous enough that a legitimately
/// large, well-sized ticket list still completes, but it stops a runaway
/// list before it burns through dozens of unattended paid sessions.
const SEQUENCE_STEP_COST_CEILING_MULTIPLIER: f64 = 20.0;

impl ExecutionDriver {
    /// Run `tasks` strictly in order inside the single worktree `wt_path`.
    ///
    /// Every task gets a brand-new agent session (so no context accumulates
    /// across the feature) but the *same* worktree and branch (so each task
    /// builds on the last, and there is nothing to merge between them). A
    /// task commits before the next one starts; the caller merges the whole
    /// branch back once, after this returns.
    ///
    /// Each task folds what it produced into `tally`, which the caller owns
    /// because those totals are the *step's*: they are seeded from the
    /// checkpoint before this runs and judged after it returns.
    ///
    /// `Err` on the first task that fails. `tally.landed()` then holds the
    /// tasks this attempt completed and committed before the
    /// failure — the caller merges that prefix to the feature branch and
    /// fails the step, or rolls the branch back when nothing landed.
    pub(crate) async fn run_tasks_loop(
        &self,
        step: StepCtx<'_>,
        spend: &mut StepSpend<'_>,
        target: RunTarget<'_>,
        wt: StepWorktree<'_>,
        plan: &TaskPlan,
        tally: &mut StepTally,
    ) -> Result<(), SequenceError> {
        let step_exec = step.step_exec;
        let tasks = &plan.tasks;

        // A targeted retry runs a subset, but the worktree it opens is cut
        // from the feature branch, which carries *every* task from the
        // previous attempt. Seed the completed record with the tasks this
        // attempt is skipping so the first running task's prompt describes
        // the tree it actually gets, rather than claiming an empty branch.
        let mut completed: Vec<CompletedTask> = plan
            .already_landed
            .iter()
            .map(|t| CompletedTask {
                id: t.id.clone(),
                title: t.title.clone(),
                files: t.files.clone(),
            })
            .collect();

        for (idx, task) in tasks.iter().enumerate() {
            if *self.cancel_watch.borrow() {
                return Err(SequenceError::Cancelled);
            }

            tracing::info!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                task_id = %task.id,
                task = idx + 1,
                of = tasks.len(),
                "sequence task start"
            );

            let files = task.files.clone();

            // A session per task, not per step: the thread id carries the
            // task id so the runtime can never hand us a cached session
            // still holding the previous task's conversation.
            let thread_id = format!("{}-{}-{}", self.f_id_str, step_exec.step_id.0, task.id);
            let run = TaskRun {
                task,
                index: idx,
                total: tasks.len(),
                completed: &completed,
                resumes_landed_work: plan.resumes_landed_work,
                plan_kind: plan.kind,
                thread_id: &thread_id,
            };

            // Telemetry row for this (task, attempt). Best-effort — a DB
            // hiccup must not fail a task whose agent work is fine — but the
            // close below runs on *every* exit, or the dashboard's live
            // "nodes" count (which counts `running` rows) would over-report
            // forever.
            let run_id = format!(
                "sr-{}-{}-{}",
                self.f_id_str,
                task.id,
                crate::paths::now_ms()
            );
            let subtask_branch =
                crate::adapters::worktree::git_ops::subtask_branch_name(&self.branch_name, wt.id);
            if let Err(e) = self.subtask_runs.subtask_run_start(
                &run_id,
                &self.f_id,
                &step_exec.id,
                &task.id,
                &thread_id,
                wt.path,
                &subtask_branch,
                crate::paths::now_ms(),
            ) {
                tracing::warn!(
                    feature_id = %self.f_id,
                    task_id = %task.id,
                    error = %e,
                    "sequence task: could not open its subtask_runs row"
                );
            }

            // Cost and tokens are the driver's, accumulated across every
            // step of the feature, so this task's own spend is only knowable
            // as the difference across its run — which is what the
            // `subtask_runs` row below wants.
            let cost_before = *spend.cost;
            let tokens_before = *spend.tokens;
            let task_res = self.run_one_task(step, spend, target, wt, run).await;

            let (status, err_msg) = match &task_res {
                Ok(_) => ("completed", None),
                Err(e) => ("failed", e.message()),
            };
            if let Err(e) = self.subtask_runs.subtask_run_finish(
                &run_id,
                status,
                *spend.cost - cost_before,
                *spend.tokens - tokens_before,
                err_msg,
                crate::paths::now_ms(),
            ) {
                tracing::warn!(
                    feature_id = %self.f_id,
                    task_id = %task.id,
                    error = %e,
                    "sequence task: could not close its subtask_runs row"
                );
            }
            let contribution = task_res?;

            // The task committed (run_one_task fails otherwise), so the
            // worktree HEAD is that commit — the checkpoint anchor a later
            // failure resets to. If even rev-parse fails, leave the task out
            // of `landed`: a retry re-running a finished task is wasteful
            // but safe, checkpointing to a wrong SHA is not.
            match self
                .sequence_git(target.machine)
                .rev_parse(wt.path, "HEAD")
                .await
            {
                Ok(sha) if !sha.is_empty() => {
                    let produced = contribution.produced();
                    self.checkpoint_landed_task(step, target.machine, &task.id, &sha, &produced)
                        .await;
                    tally.land(LandedTask {
                        id: task.id.clone(),
                        sha,
                    });
                }
                _ => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        task_id = %task.id,
                        "sequence task: committed but its HEAD could not be read; \
                         it will not be checkpointable"
                    );
                }
            }

            // Unconditional: the task's output belongs to the step whether or
            // not its commit could be pinned. Only the *resume* claim depends
            // on the SHA above.
            tally.fold(contribution);

            completed.push(CompletedTask {
                id: task.id.clone(),
                title: task.title.clone(),
                files,
            });

            let cost_ceiling = self.base_max_budget_usd() * SEQUENCE_STEP_COST_CEILING_MULTIPLIER;
            if *spend.cost > cost_ceiling {
                return Err(SequenceError::Failed(format!(
                    "sequence step: aggregate cost after {} of {} tasks reached \
                     ${:.2}, over the ${:.2} step ceiling. Work already completed and \
                     committed is preserved; the remaining tasks were not run — this \
                     usually means the task list is far larger than the feature \
                     warrants.",
                    idx + 1,
                    tasks.len(),
                    *spend.cost,
                    cost_ceiling
                )));
            }
        }

        Ok(())
    }
}
