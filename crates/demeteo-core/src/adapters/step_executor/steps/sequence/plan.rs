//! Where a `sequence` step's task list comes from.
//!
//! Preferred: an upstream step wrote it. The step names that step in
//! `task_list_from`, and the plan is read from its `task-list` artifact.
//! This is strictly better than planning inside the implement step, because
//! the decomposition then sits in front of the human gate — you approve the
//! task breakdown *before* any code is written — and it costs no agent turn.
//!
//! Fallback: no `task_list_from`. That is what a legacy `parallel` workflow
//! looks like (its steps predate the field, and we now dispatch them here),
//! so we keep the old planner turn for them rather than breaking them
//! ([`super::planner`]).

use super::context::{RunTarget, StepCtx, StepSpend};
use super::CheckpointResume;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::StepExecution;
use crate::domain::sequence::tasks::{
    apply_landed_checkpoint, extract_task_plan, select_targeted_tasks,
    task_list_json_shape_example, validate_task_plan, TaskPlan,
};

impl ExecutionDriver {
    /// Resolve the task list for this attempt.
    ///
    /// The escalation ladder mirrors the retry semantics:
    ///
    /// * **attempt 0** — take the full plan (from the artifact, or the
    ///   planner) and cache it.
    /// * **attempt 1** — reuse the cached plan and re-run only the tasks
    ///   owning the verdict's implicated files, with the feedback stamped on
    ///   each. Skipping the others is safe (and cheap): their commits are
    ///   already on the branch.
    /// * **attempt 2+** — the targeted fix did not stick. Re-resolve the full
    ///   plan; when it comes from an artifact, a gate redirect may have
    ///   revised the spec in the meantime, so re-reading picks that up.
    ///
    /// Cutting across the ladder, and only for **planner-sourced** steps:
    /// when `resume` carries landed tasks, the cached plan wins over
    /// re-resolving. A checkpoint identifies work by task id, so a plan whose
    /// ids differ from the one that produced it matches nothing — and a
    /// planner pass re-decomposed from scratch produces exactly that.
    /// Re-planning would keep the landed commits but re-pay for every one of
    /// them. A `task_list_from` step needs no such rescue (its ids are the
    /// upstream artifact's, stable across a re-read) and must not get one, or
    /// attempt 2+ would stop seeing gate revisions.
    ///
    /// `&self`: resolving a plan reads the retry context, the plan cache and
    /// the attempt rows, and writes the cache back through its repository —
    /// none of which is driver state. The `&mut self` it used to take was
    /// inherited from its caller and constrained nothing.
    pub(crate) async fn resolve_task_plan(
        &self,
        step: StepCtx<'_>,
        spend: &mut StepSpend<'_>,
        target: RunTarget<'_>,
        retry_iteration: u32,
        resume: &CheckpointResume,
    ) -> Result<TaskPlan, StepOutcome> {
        let step_exec = step.step_exec;
        let step_conf = step.step_conf;

        // Is the *previous* attempt's implementation still on the feature
        // branch? It is exactly when this step's last attempt merged — i.e.
        // the failure that sent us back here was raised by a *later* step (a
        // validate or a critic redirecting to us). A sequence step that failed
        // on its own rolled every task's commits back on the way out, leaving
        // the branch at its pre-step tip.
        //
        // Two things hang off this. A targeted retry may only skip tasks whose
        // work survived, or it silently drops them. And the tasks that do run
        // have to be *told* the tree is not empty, or a fresh session
        // reimplements code it is looking at.
        let previous_attempt_landed = retry_iteration > 0
            && self
                .retry_ctx
                .as_ref()
                .is_some_and(|rc| rc.failing_step_id != step_exec.step_id.0);

        if retry_iteration == 1 && previous_attempt_landed {
            // This step's own cached plan, never a sibling sequence step's.
            // Durable (V32): read through the repo so the targeted retry
            // works identically after a restart. An unparsable row (schema
            // drift) degrades to a full re-plan, same as a cache miss.
            let cached_for_this_step: Option<TaskPlan> = self
                .sequence_resume
                .plan_cache_get(&self.f_id, step_exec.step_id.0.as_str())
                .ok()
                .flatten()
                .and_then(|json| serde_json::from_str(&json).ok());
            if let (Some(cached), Some(rc)) = (cached_for_this_step.as_ref(), &self.retry_ctx) {
                let targeted = select_targeted_tasks(cached, &rc.feedback, &rc.implicated_files);
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    selected = targeted.tasks.len(),
                    skipped = targeted.already_landed.len(),
                    total = cached.tasks.len(),
                    "sequence step: targeted retry"
                );
                return Ok(self.skip_checkpointed_tasks(&step_exec.step_id.0, targeted, resume));
            }
        }

        // A checkpoint names the work it is skipping by task id, so this
        // attempt has to speak the same ids as the attempt that landed it.
        // A *planner* pass re-decomposes from scratch and its ids are new,
        // so the checkpoint would match nothing and every landed task would
        // be re-implemented on top of itself. The cached plan is the one
        // those ids came from, so it wins for planner-sourced steps.
        //
        // Deliberately *not* extended to `task_list_from` steps. Their ids
        // come from an upstream artifact and are stable across a re-read, so
        // the checkpoint keeps matching — and re-reading is load-bearing: a
        // gate redirect may have revised the task list since the attempt that
        // checkpointed, and preferring the cache would drop that revision on
        // the floor with nothing in the log to say so. Stability is the
        // reason to use the cache; where the artifact already provides it,
        // the artifact is the fresher source.
        let planner_sourced = step_conf
            .task_list_from
            .as_ref()
            .is_none_or(|s| s.0.is_empty());
        let cached_plan: Option<TaskPlan> = if resume.landed_ids().is_empty() || !planner_sourced {
            None
        } else {
            self.sequence_resume
                .plan_cache_get(&self.f_id, step_exec.step_id.0.as_str())
                .ok()
                .flatten()
                .and_then(|json| serde_json::from_str(&json).ok())
        };

        let mut plan = match cached_plan {
            Some(cached) => {
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    tasks = cached.tasks.len(),
                    "sequence step: resuming against the cached plan the checkpoint was \
                     recorded against"
                );
                cached
            }
            None => match step_conf
                .task_list_from
                .as_ref()
                .filter(|s| !s.0.is_empty())
            {
                Some(source_step) => {
                    self.load_task_list_artifact(source_step.0.as_str(), step.step_execs)?
                }
                None => self.run_planner_pass(step, spend, target).await?,
            },
        };

        // The plan is agent-authored whichever source it came from, so gate it
        // before it becomes N agent sessions. Non-retryable: re-running the
        // sequence step cannot fix a malformed task list — the step that wrote
        // it has to.
        if let Some(reason) = validate_task_plan(&plan) {
            return Err(StepOutcome::NonRetryable(format!(
                "sequence step: the task list is not executable — {}",
                reason
            )));
        }

        // Cache only full plans — a targeted subset must never shadow the
        // complete decomposition, or attempt 2 would re-plan from a fragment.
        // Durable (V32), stored with the attempt that produced it (the
        // step's latest V31 row). Telemetry-grade write: failure degrades
        // to a re-plan on the next targeted retry.
        let attempt_no = self
            .features
            .attempts_for_step(&step_exec.id)
            .ok()
            .and_then(|rows| rows.last().map(|a| a.attempt_no));
        match serde_json::to_string(&plan) {
            Ok(json) => {
                if let Err(e) = self.sequence_resume.plan_cache_put(
                    &self.f_id,
                    &step_exec.step_id.0,
                    &json,
                    attempt_no,
                    crate::paths::now_ms(),
                ) {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        step_id = %step_exec.step_id.0,
                        error = %e,
                        "failed to persist sequence plan cache"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    error = %e,
                    "failed to serialize task plan for the durable cache"
                );
            }
        }

        // A full re-plan still runs against whatever the last attempt left on
        // the branch, so it carries the same warning a targeted retry does.
        plan.resumes_landed_work = previous_attempt_landed;
        Ok(self.skip_checkpointed_tasks(&step_exec.step_id.0, plan, resume))
    }

    /// Drop the tasks a checkpoint already accounted for — merged to the
    /// feature branch by the mid-list failure path, or committed on the step
    /// branch by an attempt that was interrupted — so no attempt (targeted
    /// retry, full re-plan, or an environmental in-place re-run at iteration
    /// 0) re-runs and re-pays for work that already landed.
    ///
    /// Takes the resume the caller already resolved rather than reading the
    /// checkpoint again: [`CheckpointResume`] is where "can this work be put
    /// back?" was decided, and a second, independent read could answer that
    /// question differently — dropping tasks whose commits nothing is going
    /// to restore. A no-op for [`CheckpointResume::None`].
    fn skip_checkpointed_tasks(
        &self,
        step_id: &str,
        plan: TaskPlan,
        resume: &CheckpointResume,
    ) -> TaskPlan {
        let landed = resume.landed_ids();
        if landed.is_empty() {
            return plan;
        }
        // Ids that name no task in this plan buy nothing: the work stays on
        // the branch (or gets restored) but every task re-runs on top of it.
        // Silent before — the `remaining` count below looks identical to a
        // healthy resume — and it is the shape that sends a 25-task step
        // through 25 agents it had already paid for, so it says so.
        if !plan.tasks.iter().any(|t| landed.contains(&t.id)) {
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_id,
                landed = landed.len(),
                tasks = plan.tasks.len(),
                "sequence step: the checkpoint's landed task ids match nothing in this plan, so \
                 every task will re-run over work that is already committed — the plan was \
                 likely re-decomposed with fresh ids"
            );
        }
        let mut filtered = apply_landed_checkpoint(plan, landed);
        // The checkpoint exists exactly because a prefix landed — so even
        // when none of its ids match this plan (a planner re-decomposed with
        // fresh ids), the tree is not pristine and the tasks must be told so.
        filtered.resumes_landed_work = true;
        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_id,
            remaining = filtered.tasks.len(),
            landed = landed.len(),
            restored = matches!(resume, CheckpointResume::Restore { .. }),
            "sequence step: resuming after a checkpoint"
        );
        filtered
    }

    /// Read the `task-list` artifact produced by step `source_step_id`.
    fn load_task_list_artifact(
        &self,
        source_step_id: &str,
        step_execs: &[StepExecution],
    ) -> Result<TaskPlan, StepOutcome> {
        let source = step_execs
            .iter()
            .find(|s| s.step_id.0 == source_step_id)
            .ok_or_else(|| {
                StepOutcome::NonRetryable(format!(
                    "sequence step: `task_list_from` names step '{}', which this workflow does \
                     not contain.",
                    source_step_id
                ))
            })?;

        let refs: Vec<String> = if !source.artifact_paths.is_empty() {
            source.artifact_paths.clone()
        } else {
            source.artifact_path.iter().cloned().collect()
        };
        if refs.is_empty() {
            return Err(StepOutcome::Failed(format!(
                "sequence step: step '{}' produced no artifacts, so there is no task list to \
                 execute. It must write the task list to `artifacts/task-list.json` and declare \
                 it as a `task-list` artifact.",
                source_step_id
            )));
        }

        // Prefer the ref that actually looks like the task list; fall back to
        // trying each one, since an agent may have named the file slightly
        // differently than the declaration implies.
        let mut candidates: Vec<&String> = refs
            .iter()
            .filter(|r| r.to_lowercase().contains("task-list"))
            .collect();
        candidates.extend(
            refs.iter()
                .filter(|r| !r.to_lowercase().contains("task-list")),
        );

        for reference in candidates {
            let Ok(body) = self.artifacts.get(reference) else {
                continue;
            };
            if let Some(plan) = extract_task_plan(&body) {
                if !plan.tasks.is_empty() {
                    return Ok(plan);
                }
            }
        }

        Err(StepOutcome::Failed(format!(
            "sequence step: could not read a task list from step '{}'. It must write a JSON \
             object of the form {} to `artifacts/task-list.json`.",
            source_step_id,
            task_list_json_shape_example(false)
        )))
    }
}
