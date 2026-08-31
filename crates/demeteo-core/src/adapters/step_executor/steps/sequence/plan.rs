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
    apply_landed_checkpoint, extract_task_plan, is_rework_plan, reject_unexecutable_plan,
    select_targeted_tasks, task_list_json_shape_example, PlanKind, PlanRejection, TaskPlan,
};

impl ExecutionDriver {
    /// Resolve the task list for this attempt.
    ///
    /// Two sources, and they answer a retry completely differently.
    ///
    /// **A step with a producer** (`task_list_from`) asks it. When a verdict
    /// from behind this step sent the run back, the producer has already
    /// re-run in rework mode and written a *delta* list — the four tickets
    /// that close the verdict, not the twenty-five that built the feature.
    /// So this step runs the list it is given, whole, and reports the
    /// previous cycles as already landed. There is no selection to make
    /// here: the producer made it, with the verdict text, the spec and the
    /// diff in hand, which is strictly more than a file-overlap heuristic
    /// can know.
    ///
    /// **A step with no producer** — a legacy `parallel` workflow, whose
    /// steps predate the field — has nobody to ask, so it keeps the old
    /// escalation ladder: plan at attempt 0, re-run the tasks owning the
    /// verdict's implicated files at attempt 1
    /// ([`select_targeted_tasks`]), re-plan whole at attempt 2+. That
    /// ladder is why this used to cost a 25-ticket feature three full runs
    /// of itself; it survives only where nothing better is available.
    ///
    /// Cutting across both, and only for **planner-sourced** steps: when
    /// `resume` carries landed tasks, the cached plan wins over
    /// re-resolving. A checkpoint identifies work by task id, so a plan
    /// whose ids differ from the one that produced it matches nothing — and
    /// a planner pass re-decomposed from scratch produces exactly that.
    /// Re-planning would keep the landed commits but re-pay for every one
    /// of them. A `task_list_from` step needs no such rescue: its ids are
    /// the artifact's, and a rework list's non-matching ids are the point,
    /// not a drift to be rescued from.
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

        // This step's own cached plan, never a sibling sequence step's.
        // Durable (V32): read through the repo so a retry behaves
        // identically after a restart. An unparsable row (schema drift)
        // degrades to a full re-resolve, same as a cache miss.
        let cached: Option<TaskPlan> = self
            .sequence_resume
            .plan_cache_get(&self.f_id, step_exec.step_id.0.as_str())
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok());

        let planner_sourced = step_conf
            .task_list_from
            .as_ref()
            .is_none_or(|s| s.0.is_empty());

        // A step with no producer has nobody to ask for a delta, so the old
        // ladder is still the best it can do: one targeted retry off the
        // cached plan, then a full re-plan. Only reachable from a legacy
        // `parallel` workflow — every `task_list_from` step takes the
        // producer path below.
        if planner_sourced && retry_iteration == 1 && previous_attempt_landed {
            if let (Some(cached), Some(rc)) = (cached.as_ref(), &self.retry_ctx) {
                let targeted = select_targeted_tasks(cached, &rc.feedback, &rc.implicated_files);
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    selected = targeted.tasks.len(),
                    skipped = targeted.already_landed.len(),
                    total = cached.tasks.len(),
                    "sequence step: targeted retry (no task-list producer to ask for a delta)"
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
        let cached_plan: Option<TaskPlan> = if resume.landed_ids().is_empty() || !planner_sourced {
            None
        } else {
            cached.clone()
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
        // before it becomes N agent sessions. Re-running *this* step cannot
        // fix a malformed task list — the step that wrote it has to — so send
        // it there. Every rule `validate_task_plan` enforces (blank id,
        // duplicate id, self-dependency, forward dependency) is a defect the
        // producer can repair from the message alone, and the redirect budget
        // bounds a producer that keeps writing bad lists.
        //
        // With no producer to ask there is nowhere to send it, and a planner
        // pass that re-decomposes from scratch has already had its go — so
        // that case stays terminal.
        if let Some(rejection) = reject_unexecutable_plan(
            &plan,
            step_conf
                .task_list_from
                .as_ref()
                .filter(|s| !s.0.is_empty()),
        ) {
            return Err(match rejection {
                PlanRejection::ProducerMustFix { producer, reason } => {
                    StepOutcome::ProducerFault { producer, reason }
                }
                PlanRejection::Terminal { reason } => StepOutcome::NonRetryable(reason),
            });
        }

        // Is the list we just read a *delta* against the previous cycle, or
        // a fresh whole decomposition?
        //
        // Only asked when the run is already in a rework cycle by graph
        // position — a verdict from behind this step's producer sent it
        // back — because that is the only way a producer could have written
        // one. A gate revision reaching here re-reads a *greenfield* list
        // and must keep being treated as one, however its ids compare.
        let in_rework_cycle = self.rework_mode(step_conf).is_rework();

        // Nothing to run. Two very different situations wear the same shape,
        // and they must not end the same way — so this is decided here,
        // where the cycle is known, rather than at the read. Deliberately
        // ahead of the cache write below: an empty list must not overwrite
        // the decomposition it was a (non-)delta against.
        if plan.tasks.is_empty() {
            return Err(self.empty_task_list_outcome(
                step_conf
                    .task_list_from
                    .as_ref()
                    .map(|s| s.0.as_str())
                    .unwrap_or("the planner"),
                &plan,
                in_rework_cycle,
            ));
        }

        let is_delta =
            in_rework_cycle && !planner_sourced && is_rework_plan(&plan, cached.as_ref());

        if is_delta {
            // Every task runs. The producer already chose which four of the
            // twenty-five matter, holding the verdict, the spec and the diff
            // — and no selection made here from `files` alone can improve on
            // that. `already_landed` carries the cycles it is a delta
            // against so each running agent's `{{completed_tasks}}` names
            // the work sitting in the worktree it opens.
            let previous = cached.as_ref();
            plan.kind = PlanKind::Rework;
            plan.cycle = previous.map(|c| c.cycle + 1).unwrap_or(1);
            plan.history = previous.map(|c| c.close_cycle()).unwrap_or_default();
            plan.already_landed = plan.all_prior_tasks();
            plan.resumes_landed_work = true;
            tracing::info!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                cycle = plan.cycle,
                tasks = plan.tasks.len(),
                landed = plan.already_landed.len(),
                "sequence step: rework cycle — running the producer's delta"
            );
        } else if !planner_sourced && in_rework_cycle {
            // A rework cycle whose producer handed back a whole list
            // anyway: it declared no `rework_prompt_template`, or the agent
            // ignored it. Running it is correct — every task re-runs over
            // its own committed output, which is what happened before this
            // change existed — but it is the expensive shape, so it says so
            // rather than looking like a healthy delta in the log.
            tracing::warn!(
                feature_id = %self.f_id,
                step_id = %step_exec.step_id.0,
                tasks = plan.tasks.len(),
                producer = %step_conf
                    .task_list_from
                    .as_ref()
                    .map(|s| s.0.as_str())
                    .unwrap_or_default(),
                "sequence step: rework cycle, but the task list is a whole decomposition, not a                  delta — every task will re-run over work already on the branch. The producer                  likely declares no `rework_prompt_template`."
            );
        }

        // Cache the plan. A targeted subset must never shadow the complete
        // decomposition, or the next re-plan starts from a fragment — but a
        // rework delta is not a subset, it is this cycle's whole list, and
        // it carries every earlier cycle in `history`. Durable (V32),
        // stored with the attempt that produced it (the step's latest V31
        // row). Telemetry-grade write: failure degrades to a re-plan on the
        // next retry.
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

        // A whole list re-run still runs against whatever the last attempt
        // left on the branch, so it carries the same warning a delta does.
        // Already true for a delta, and `||` keeps it that way rather than
        // letting a `false` here overwrite it.
        plan.resumes_landed_work |= previous_attempt_landed;
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

    /// How a step reacts to a task list that parses but holds no tasks.
    ///
    /// **In a rework cycle this is a sanctioned answer, not a fault.** The
    /// rework prompt tells the producer to emit nothing when the review it
    /// is scoping named no defect an implementation ticket can close — a
    /// criterion the project's harness cannot evidence, say. Retrying
    /// cannot help: the producer has already looked at the report and the
    /// code and concluded there is no ticket to write, and asking again
    /// spends another cycle to be told the same thing (or, worse, to be
    /// handed invented work). So it parks on the synthetic gate and hands
    /// the producer's own reason to the human, who is the only one who
    /// *can* act on it.
    ///
    /// It used to end the run instead, which was the same sentence with
    /// the second half missing: the reason went into a database column and
    /// the decision was in front of nobody.
    ///
    /// **Outside a rework cycle it is a fault**, and a retryable one: a
    /// decomposition step that returned no tickets for a feature that has
    /// not been built yet simply failed at its job, and re-running it is a
    /// reasonable thing to try.
    fn empty_task_list_outcome(
        &self,
        producer: &str,
        plan: &TaskPlan,
        in_rework_cycle: bool,
    ) -> StepOutcome {
        if in_rework_cycle || plan.kind == PlanKind::Rework {
            let reason = plan
                .notes
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .unwrap_or("It gave no reason.");
            tracing::info!(
                feature_id = %self.f_id,
                producer = %producer,
                "sequence step: rework cycle scoped to zero tickets — stopping the loop"
            );
            StepOutcome::AwaitHumanDecision(crate::domain::step_park::HumanPark {
                reason: format!(
                    "Step '{}' scoped this rework cycle to zero tickets — it found nothing in \
                     the review feedback that an implementation ticket can fix, so there is no \
                     code change to make and re-running it would only ask the same question \
                     again.\n\nIts stated reason:\n\n{}\n\nApprove to accept that there is \
                     nothing to implement and let the run continue, or redirect to send '{}' \
                     back with different direction.",
                    producer, reason, producer
                ),
                // The producer is a real target here, unlike the resume
                // guard's park: it emitted nothing and can be told what to
                // emit instead.
                redirect_to: Some(crate::domain::ids::StepId::from(producer.to_string())),
            })
        } else {
            StepOutcome::Failed(format!(
                "sequence step: step '{}' wrote a task list containing no tickets, so there is \
                 nothing to implement. A greenfield decomposition must emit at least one ticket \
                 covering the spec's acceptance criteria.",
                producer
            ))
        }
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

        // A list that parses but carries no tasks is *not* a read failure,
        // and conflating the two is how a deliberate "nothing to do here"
        // got reported as malformed JSON. Keep the first one so the caller
        // can tell them apart, but keep looking: an empty list in one
        // candidate must not shadow a real one in the next.
        let mut parsed_empty: Option<TaskPlan> = None;
        for reference in candidates {
            let Ok(body) = self.artifacts.get(reference) else {
                continue;
            };
            if let Some(plan) = extract_task_plan(&body) {
                if !plan.tasks.is_empty() {
                    return Ok(plan);
                }
                parsed_empty.get_or_insert(plan);
            }
        }
        if let Some(plan) = parsed_empty {
            return Ok(plan);
        }

        Err(StepOutcome::Failed(format!(
            "sequence step: could not read a task list from step '{}'. It must write a JSON \
             object of the form {} to `artifacts/task-list.json`.",
            source_step_id,
            task_list_json_shape_example(false)
        )))
    }
}
