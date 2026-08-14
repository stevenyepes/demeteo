//! The legacy planner turn: an agent decomposes the feature into the ordered
//! task list, in its own read-only worktree.
//!
//! Only reached when the step declares no `task_list_from` — see [`super::plan`]
//! for why that is the fallback and not the preferred source.

use crate::adapters::step_executor::artifacts::{
    resolve_attached_artifacts, resolve_attached_user_attachments,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::platform_context::place_platform_context;
use crate::domain::sequence::tasks::{extract_task_plan, task_list_json_shape_example, TaskPlan};
use crate::ports::agent_runtime::AgentContext;

use super::context::{RunTarget, StepCtx, StepSpend};

impl ExecutionDriver {
    /// Legacy fallback: decompose the feature with a planner agent turn.
    ///
    /// Only reached when the step declares no `task_list_from` — i.e. a
    /// workflow authored against the old `parallel` kind. New workflows put
    /// the task list in front of the gate instead; see the module docs.
    pub(crate) async fn run_planner_pass(
        &self,
        step: StepCtx<'_>,
        spend: &mut StepSpend<'_>,
        target: RunTarget<'_>,
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
        let task_shape = task_list_json_shape_example(true);
        let planner_prompt = format!(
            "You are a planning agent. Break the following feature into an ordered list of \
             tasks that will be implemented ONE AT A TIME, in the order you give, each by a \
             separate agent with a fresh context, each committing before the next starts.\n\n\
             Feature: {feature_desc}\n\
             Repositories in scope: {repo_list}\n\n\
             Read any attached artifacts (e.g. the spec) for context. Then emit a single JSON \
             object, in a ```json ... ``` fence, of the form:\n\
             {task_shape}\n\n\
             Constraints:\n\
             - Size each task so ONE agent can complete it in ONE session: small enough that \
             reading the relevant code, implementing, and testing all fit comfortably. \
             Guideline: 1–3 closely related files. If a task's description needs \"and then \
             also…\", split it. There is no upper limit on the task count.\n\
             - Prefer vertical slices that are verifiable on their own over horizontal layers \
             like \"all the types\".\n\
             - Order matters: each task may rely on the ones before it being done and \
             committed. Declare the ids of earlier tasks a task builds on in `blocked_by`.\n\
             - Task IDs must be kebab-case, unique, and stable.\n\
             - `files` lists what the task is expected to touch. Tasks MAY share files — a later \
             task building on an earlier one's file is normal and expected.\n\
             - `acceptance` is 1–3 binary pass/fail statements; `test_command` is what proves \
             the task done. Write it as a POSIX shell body on every platform, whatever shell \
             your own commands run in: Demeteo executes it through `sh`, and on Windows that \
             is the bash Git for Windows installs.\n\
             - If no decomposition makes sense (the work is small), return a single task with id \
             `task-1` that does the whole thing.\
             {retry_note_constraint}\
             {retry_section}",
        );
        let planner_prompt = resolve_attached_artifacts(
            &planner_prompt,
            step.step_execs,
            step.step_index,
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

        // The planner both runs commands and *writes* them, and the two do not
        // share an interpreter. This block covers the first: what its own
        // command tool will execute. The second is covered in the constraints
        // above, and cannot be covered here — a `test_command` outlives this
        // turn in a cached plan, and the agent kind that later runs it is
        // editable between attempts, so the only safe authorship rule is the
        // one that holds for every consumer.
        let planner_prompt = format!(
            "{}{}",
            place_platform_context(
                target.platform,
                self.registry.windows_agent_shell_for(target.agent_kind),
                crate::shared::win::quotable_bash_path().as_deref(),
                &planner_prompt,
            )
            .prefix,
            planner_prompt
        );

        let planner_env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), target.machine).await;
        let planner_binary = self
            .registry
            .runtime_for(target.agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| target.agent_kind.to_string());

        let planner_ctx = AgentContext {
            thread_id: planner_thread_id.clone(),
            machine_id: target.machine.to_string(),
            binary: planner_binary,
            args: vec![],
            env: planner_env,
            cwd: planner_wt_path.clone(),
            model: target.override_model.map(str::to_string),
            // Decomposing the spec into an ordered task list is real agent
            // work — it inherits the step's resolved effort.
            effort: Some(target.effort),
            title: Some("plan".to_string()),
            platform: target.platform,
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
            bare_mode: true,
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
            res = self.registry.get_or_spawn(&planner_thread_id, target.agent_kind, planner_ctx) => Some(res),
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
                format!(
                    "Your previous response could not be parsed as the required task list. \
                     Reply with ONLY a single JSON object — no prose, no markdown outside the \
                     fence — of the form:\n\
                     ```json\n\
                     {task_shape}\n\
                     ```"
                )
            };

            let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
                &*planner_session,
                &prompt,
                timeouts,
                Some(self.cancel_watch.clone()),
                target.machine,
                &*self.exec,
                target.override_model.map(str::to_string),
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
                    *spend.cost += outcome.cost_usd;
                    *spend.tokens += outcome.tokens;
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
