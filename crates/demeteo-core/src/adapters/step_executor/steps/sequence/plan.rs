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
//! so we keep the old planner turn for them rather than breaking them.

use super::tasks::{
    apply_landed_checkpoint, extract_task_plan, select_targeted_tasks, validate_task_plan, TaskPlan,
};
use crate::adapters::step_executor::artifacts::{
    resolve_attached_artifacts, resolve_attached_user_attachments,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::agent_runtime::AgentContext;

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
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn resolve_task_plan(
        &mut self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        retry_iteration: u32,
        agent_kind: &str,
        override_model: &Option<String>,
        machine_str: &str,
        step_execs: &[StepExecution],
        step_index: usize,
    ) -> Result<TaskPlan, StepOutcome> {
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
            let cached_for_this_step = self.cached_plans.get(step_exec.step_id.0.as_str());
            if let (Some(cached), Some(rc)) = (cached_for_this_step, &self.retry_ctx) {
                let targeted = select_targeted_tasks(cached, &rc.feedback, &rc.implicated_files);
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_exec.step_id.0,
                    selected = targeted.tasks.len(),
                    skipped = targeted.already_landed.len(),
                    total = cached.tasks.len(),
                    "sequence step: targeted retry"
                );
                return Ok(self.skip_checkpointed_tasks(&step_exec.step_id.0, targeted));
            }
        }

        let mut plan = match step_conf
            .task_list_from
            .as_ref()
            .filter(|s| !s.0.is_empty())
        {
            Some(source_step) => {
                self.load_task_list_artifact(source_step.0.as_str(), step_execs)?
            }
            None => {
                self.run_planner_pass(
                    accumulated_cost,
                    accumulated_tokens,
                    agent_kind,
                    override_model,
                    self.resolve_step_effort(step_conf),
                    machine_str,
                    step_execs,
                    step_index,
                )
                .await?
            }
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
        self.cached_plans
            .insert(step_exec.step_id.0.clone(), plan.clone());

        // A full re-plan still runs against whatever the last attempt left on
        // the branch, so it carries the same warning a targeted retry does.
        plan.resumes_landed_work = previous_attempt_landed;
        Ok(self.skip_checkpointed_tasks(&step_exec.step_id.0, plan))
    }

    /// Drop the tasks a mid-list checkpoint already merged to the feature
    /// branch (see `handle_sequence_step`'s failure path), so no attempt —
    /// targeted retry, full re-plan, or an environmental in-place re-run at
    /// iteration 0 — re-runs and re-pays for work that already landed. A
    /// no-op unless this step checkpointed.
    fn skip_checkpointed_tasks(&self, step_id: &str, plan: TaskPlan) -> TaskPlan {
        match self.sequence_checkpoints.get(step_id) {
            Some(landed) if !landed.is_empty() => {
                let mut filtered = apply_landed_checkpoint(plan, landed);
                // The checkpoint exists exactly because a prefix *merged* —
                // so even when none of its ids match this plan (a legacy
                // planner re-decomposed with fresh ids), the branch is not
                // pristine and the tasks must be told so.
                filtered.resumes_landed_work = true;
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_id,
                    remaining = filtered.tasks.len(),
                    landed = landed.len(),
                    "sequence step: resuming after a mid-list checkpoint"
                );
                filtered
            }
            _ => plan,
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
             object of the form {{\"tasks\": [{{\"id\": \"task-1\", \"title\": \"...\", \
             \"description\": \"...\", \"files\": [\"src/foo.rs\"], \"test_command\": \"...\"}}]}} \
             to `artifacts/task-list.json`.",
            source_step_id
        )))
    }

    /// Legacy fallback: decompose the feature with a planner agent turn.
    ///
    /// Only reached when the step declares no `task_list_from` — i.e. a
    /// workflow authored against the old `parallel` kind. New workflows put
    /// the task list in front of the gate instead; see the module docs.
    #[allow(clippy::too_many_arguments)]
    async fn run_planner_pass(
        &self,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        agent_kind: &str,
        override_model: &Option<String>,
        effort: crate::domain::models::EffortLevel,
        machine_str: &str,
        step_execs: &[StepExecution],
        step_index: usize,
    ) -> Result<TaskPlan, StepOutcome> {
        let planner_thread_id = format!("{}-planner", self.f_id_str);
        let feature_desc = self.base_ctx.get("feature_description").to_string();
        let repo_list = self.base_ctx.get("repo_list").to_string();

        // Isolated worktree: the planner only reads, but an accidental write
        // must not reach the live feature branch.
        let planner_wt_id = format!("{}-planner-pass", self.f_id_str);
        let planner_wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &planner_wt_id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return Err(StepOutcome::Environmental(format!(
                    "sequence step: planner worktree provision failed: {}",
                    e
                )))
            }
        };

        let retry_section =
            crate::adapters::step_executor::steps::agent::format_retry_feedback_section(
                self.retry_ctx.as_ref(),
            );
        let retry_note_constraint = if retry_section.is_empty() {
            String::new()
        } else {
            "\n- `retry_note`: Add targeted, task-specific guidance based on the previous \
             failure. Set to `null` if the feedback doesn't apply to this task.\n"
                .to_string()
        };
        // Note the ordering language: unlike the old parallel planner, tasks
        // run sequentially on one branch, so they must be ordered by
        // dependency and may share files.
        let planner_prompt = format!(
            "You are a planning agent. Break the following feature into a small, ordered list \
             of tasks that will be implemented ONE AT A TIME, in the order you give, each by a \
             separate agent, each committing before the next starts.\n\n\
             Feature: {feature_desc}\n\
             Repositories in scope: {repo_list}\n\n\
             Read any attached artifacts (e.g. the spec) for context. Then emit a single JSON \
             object, in a ```json ... ``` fence, of the form:\n\
             {{\"tasks\": [{{\"id\": \"task-1\", \"title\": \"...\", \"description\": \"...\", \
             \"files\": [\"src/foo.rs\"], \"test_command\": \"...\", \"retry_note\": null}}]}}\n\n\
             Constraints:\n\
             - 2 to 5 tasks. Aim for the smallest set that covers the work end-to-end.\n\
             - Order matters: each task may rely on the ones before it being done and committed. \
             Put foundational work (types, schema, shared helpers) first.\n\
             - Task IDs must be kebab-case, unique, and stable.\n\
             - `files` lists what the task is expected to touch. Tasks MAY share files — a later \
             task building on an earlier one's file is normal and expected.\n\
             - If no decomposition makes sense (the work is small), return a single task with id \
             `task-1` that does the whole thing.\
             {retry_note_constraint}\
             {retry_section}",
        );
        let planner_prompt = resolve_attached_artifacts(
            &planner_prompt,
            step_execs,
            step_index,
            &*self.artifacts,
            &self.steps,
        );

        let planner_feature_attachments: Vec<crate::domain::attachment::AttachedFile> = self
            .features
            .get(&self.f_id)
            .ok()
            .flatten()
            .map(|f| f.attachments.clone())
            .unwrap_or_default();
        let planner_prompt = resolve_attached_user_attachments(
            &planner_prompt,
            self.f_id.as_str(),
            &planner_feature_attachments,
            &*self.attachments,
            None,
        );

        let planner_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;
        let planner_binary = self
            .registry
            .runtime_for(agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.to_string());

        let planner_ctx = AgentContext {
            thread_id: planner_thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary: planner_binary,
            args: vec![],
            env: planner_env,
            cwd: planner_wt_path.clone(),
            model: override_model.clone(),
            // Decomposing the spec into an ordered task list is real agent
            // work — it inherits the step's resolved effort.
            effort: Some(effort),
            title: Some("plan".to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            // Reads the codebase and emits JSON. Shell is allowed for
            // exploration (grep, find, git log); writes are denied, and the
            // worktree is the real isolation.
            permissions: crate::domain::permission::resolve_profile(
                crate::domain::permission::StepCapability::ReadOnly,
                false, // no network
                true,  // allow shell for codebase exploration
            ),
            bare_mode: agent_kind == "claude-code",
            // Full toolset — the planner explores the codebase before
            // decomposing. The cap is anti-runaway only: decomposition
            // should never take 50 round trips.
            tool_allowlist: None,
            max_turns: Some(50),
            // Explores the codebase before decomposing into a task list.
            max_budget_usd: self.role_max_budget_usd(Self::BUDGET_FRACTION_PLANNER),
        };

        let mut cancel_watch = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = self.registry.get_or_spawn(&planner_thread_id, agent_kind, planner_ctx) => Some(res),
            _ = cancel_watch.changed() => None,
        };

        let planner_session = match spawn_res {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                self.cleanup_planner(&planner_thread_id, &planner_wt_id)
                    .await;
                return Err(StepOutcome::Environmental(format!(
                    "sequence step: planner spawn failed: {:?}",
                    e
                )));
            }
            None => {
                self.cleanup_planner(&planner_thread_id, &planner_wt_id)
                    .await;
                return Err(StepOutcome::Cancelled);
            }
        };

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());

        // The planner's output is machine-consumed, but CLI agents stream free
        // text and sometimes wrap or precede the JSON with prose. Try once, and
        // on a parse miss re-ask the *same* session with a strict JSON-only
        // correction prompt before giving up.
        const PLANNER_MAX_ATTEMPTS: usize = 2;
        let mut last_text = String::new();
        let mut parsed: Option<TaskPlan> = None;

        for attempt in 0..PLANNER_MAX_ATTEMPTS {
            let prompt = if attempt == 0 {
                planner_prompt.clone()
            } else {
                "Your previous response could not be parsed as the required task list. Reply \
                 with ONLY a single JSON object — no prose, no markdown outside the fence — of \
                 the form:\n\
                 ```json\n\
                 {\"tasks\": [{\"id\": \"task-1\", \"title\": \"...\", \"description\": \"...\", \
                 \"files\": [\"src/foo.rs\"], \"test_command\": \"...\", \"retry_note\": null}]}\n\
                 ```"
                .to_string()
            };

            let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
                &*planner_session,
                &prompt,
                timeouts,
                Some(self.cancel_watch.clone()),
                machine_str,
                &*self.exec,
                override_model.clone(),
                self.pricing.clone(),
                |_event| {},
            )
            .await;

            last_text = match turn_res {
                crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                    self.cleanup_planner(&planner_thread_id, &planner_wt_id)
                        .await;
                    return Err(StepOutcome::Cancelled);
                }
                crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                    self.cleanup_planner(&planner_thread_id, &planner_wt_id)
                        .await;
                    return Err(StepOutcome::Failed(format!(
                        "sequence step: planner failed: {}",
                        descriptive
                    )));
                }
                crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
                    self.cleanup_planner(&planner_thread_id, &planner_wt_id)
                        .await;
                    return Err(StepOutcome::Environmental(format!(
                        "sequence step: planner failed: {}",
                        descriptive
                    )));
                }
                crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                    *accumulated_cost += outcome.cost_usd;
                    *accumulated_tokens += outcome.tokens;
                    outcome.text
                }
            };

            match extract_task_plan(&last_text) {
                Some(p) if !p.tasks.is_empty() => {
                    parsed = Some(p);
                    break;
                }
                _ => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        attempt = attempt + 1,
                        max = PLANNER_MAX_ATTEMPTS,
                        "sequence step: planner produced no valid task list"
                    );
                }
            }
        }

        self.cleanup_planner(&planner_thread_id, &planner_wt_id)
            .await;

        parsed.ok_or_else(|| {
            StepOutcome::Failed(format!(
                "sequence step: planner did not return a valid task list after {} attempts. The \
                 agent's last response was: {}",
                PLANNER_MAX_ATTEMPTS,
                if last_text.len() > 500 {
                    let head: String = last_text.chars().take(500).collect();
                    format!("{}…(truncated)", head)
                } else {
                    last_text
                }
            ))
        })
    }

    async fn cleanup_planner(&self, thread_id: &str, wt_id: &str) {
        crate::adapters::agent::event_stream::cleanup_subtask(
            &self.registry,
            &self.git_ops,
            self.machine_id_opt.as_deref(),
            &self.target_dir,
            &self.branch_name,
            wt_id,
            thread_id,
        )
        .await;
    }
}
