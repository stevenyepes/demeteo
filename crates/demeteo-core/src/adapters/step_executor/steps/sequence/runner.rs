use std::time::Instant;

use super::tasks::{PlannedTask, TaskPlan};
use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, inject_artifact_contract, read_worktree_file,
    resolve_attached_artifacts, resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::paths;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::notification::DomainEvent;

use crate::adapters::step_executor::steps::agent::{
    append_retry_feedback_section, format_retry_feedback_section, template_uses_retry_section,
};

/// What one finished task contributed, carried forward so the next task's
/// agent — a *fresh* session with none of the previous conversation — can
/// be told what already landed. Without this, task N re-derives (or worse,
/// redoes) task N-1's work, which is the "implement says it's already done"
/// half of the standoff this design replaces.
struct CompletedTask {
    id: String,
    title: String,
    files: Vec<String>,
}

/// A task this attempt finished *and committed*, with the worktree HEAD its
/// commit produced. When a later task fails, the caller resets the worktree
/// to the last entry's `sha` (discarding the failed task's debris, including
/// any commits its agent made itself) and merges the prefix to the feature
/// branch, so the completed tasks' work — already paid for — survives the
/// failure and the retry runs only the remainder.
pub(crate) struct LandedTask {
    pub id: String,
    pub sha: String,
}

impl ExecutionDriver {
    /// Run `tasks` strictly in order inside the single worktree `wt_path`.
    ///
    /// Every task gets a brand-new agent session (so no context accumulates
    /// across the feature) but the *same* worktree and branch (so each task
    /// builds on the last, and there is nothing to merge between them). A
    /// task commits before the next one starts; the caller merges the whole
    /// branch back once, after this returns.
    ///
    /// `Err((message, environmental))` on the first task that fails. `landed`
    /// then holds the tasks this attempt completed and committed before the
    /// failure — the caller merges that prefix to the feature branch and
    /// fails the step, or rolls the branch back when nothing landed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_tasks_loop(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
        plan: &TaskPlan,
        machine_str: &str,
        wt_id: &str,
        wt_path: &str,
        agent_kind: &str,
        override_model: &Option<String>,
        all_artifact_refs: &mut Vec<String>,
        satisfied_decls: &mut std::collections::HashSet<String>,
        landed: &mut Vec<LandedTask>,
    ) -> Result<(), (String, bool)> {
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
                return Err(("Execution cancelled by user".to_string(), false));
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
                crate::adapters::worktree::git_ops::subtask_branch_name(&self.branch_name, wt_id);
            if let Err(e) = self.subtask_runs.subtask_run_start(
                &run_id,
                &self.f_id,
                &step_exec.id,
                &task.id,
                &thread_id,
                wt_path,
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

            let cost_before = *accumulated_cost;
            let tokens_before = *accumulated_tokens;
            let task_res = self
                .run_one_task(
                    step_exec,
                    step_conf,
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                    step_index,
                    step_execs,
                    task,
                    idx,
                    tasks.len(),
                    &completed,
                    plan.resumes_landed_work,
                    &thread_id,
                    machine_str,
                    wt_path,
                    agent_kind,
                    override_model,
                    all_artifact_refs,
                    satisfied_decls,
                )
                .await;

            let (status, err_msg) = match &task_res {
                Ok(()) => ("completed", None),
                Err((msg, _)) => ("failed", Some(msg.as_str())),
            };
            if let Err(e) = self.subtask_runs.subtask_run_finish(
                &run_id,
                status,
                *accumulated_cost - cost_before,
                *accumulated_tokens - tokens_before,
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
            task_res?;

            // The task committed (run_one_task fails otherwise), so the
            // worktree HEAD is that commit — the checkpoint anchor a later
            // failure resets to. If even rev-parse fails, leave the task out
            // of `landed`: a retry re-running a finished task is wasteful
            // but safe, checkpointing to a wrong SHA is not.
            match self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} rev-parse HEAD",
                        paths::shell_escape_posix(wt_path)
                    ),
                )
                .await
            {
                Ok(sha) if !sha.trim().is_empty() => landed.push(LandedTask {
                    id: task.id.clone(),
                    sha: sha.trim().to_string(),
                }),
                _ => {
                    tracing::warn!(
                        feature_id = %self.f_id,
                        task_id = %task.id,
                        "sequence task: committed but its HEAD could not be read; \
                         it will not be checkpointable"
                    );
                }
            }

            completed.push(CompletedTask {
                id: task.id.clone(),
                title: task.title.clone(),
                files,
            });
        }

        Ok(())
    }

    /// One task: fresh session, one turn, diff guard, commit. Never merges
    /// and never touches the feature branch — the caller owns that.
    #[allow(clippy::too_many_arguments)]
    async fn run_one_task(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
        task: &PlannedTask,
        task_idx: usize,
        task_total: usize,
        completed: &[CompletedTask],
        resumes_landed_work: bool,
        thread_id: &str,
        machine_str: &str,
        wt_path: &str,
        agent_kind: &str,
        override_model: &Option<String>,
        all_artifact_refs: &mut Vec<String>,
        satisfied_decls: &mut std::collections::HashSet<String>,
    ) -> Result<(), (String, bool)> {
        let snapshot = WorktreeSnapshot::capture(&*self.exec, machine_str, wt_path).await;
        // The worktree's HEAD *before* the agent runs. The snapshot delta
        // misses work the agent committed itself, and diffing against the
        // worktree's own HEAD afterwards would miss it too — the commit moved
        // HEAD. Pinning the pre-turn commit is what lets the fallback below
        // see it. Diffing against the feature branch instead would over-report
        // here in a way it could not in the old parallel step: this worktree
        // already carries every earlier task's commits.
        let pre_head = self
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse HEAD",
                    paths::shell_escape_posix(wt_path)
                ),
            )
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let prompt = self
            .build_task_prompt(
                step_conf,
                step_index,
                step_execs,
                task,
                task_idx,
                task_total,
                completed,
                resumes_landed_work,
                machine_str,
                wt_path,
            )
            .await;

        let session = self
            .spawn_sequence_session(
                thread_id,
                &task.title,
                machine_str,
                wt_path,
                agent_kind,
                override_model,
                self.resolve_step_effort(step_conf),
            )
            .await
            .map_err(|(msg, environmental)| {
                (
                    format!("sequence task '{}': {}", task.id, msg),
                    environmental,
                )
            })?;

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let base_cost = *accumulated_cost;
        let base_tokens = *accumulated_tokens;

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            &*session,
            &prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            machine_str,
            &*self.exec,
            override_model.clone(),
            self.pricing.clone(),
            |event| {
                if let AgentEvent::Text { delta } = event {
                    let _ = self.notif.emit(&DomainEvent::AgentStream {
                        feature_id: self.f_id.clone(),
                        step_execution_id: step_exec.id.clone(),
                        content: delta.clone(),
                    });
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "running".into(),
                        cost_usd: Some(base_cost),
                        tokens: Some(base_tokens),
                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await;

        let mut produced_artifacts: Vec<Artifact> = Vec::new();
        let turn_err: Option<(String, bool)> = match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                produced_artifacts = outcome.produced_artifacts;
                None
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => Some((
                format!("sequence task '{}': agent error: {}", task.id, descriptive),
                false,
            )),
            crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => Some((
                format!("sequence task '{}': agent error: {}", task.id, descriptive),
                true,
            )),
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                Some(("Execution cancelled by user".to_string(), false))
            }
        };

        // The session's work is done either way — a task never needs its
        // conversation again, and leaving it alive would keep the model's
        // context (and the runtime process) around for the whole step.
        let _ = self.registry.kill(thread_id).await;

        if let Some(err) = turn_err {
            return Err(err);
        }

        // The step's worktree must still exist. If it does not, the agent's
        // writes were never committed (that happens below) and are gone —
        // report it rather than capturing an empty delta and moving on. See
        // `WorktreeSnapshot::worktree_is_missing`.
        if WorktreeSnapshot::worktree_is_missing(&*self.exec, machine_str, wt_path).await {
            return Err((
                format!(
                    "sequence task '{}': the step's worktree '{}' disappeared while the agent \
                     was running — its uncommitted changes are unrecoverable.",
                    task.id, wt_path
                ),
                true,
            ));
        }

        // Artifact capture: snapshot delta, falling back to a diff against
        // the pre-turn HEAD when the snapshot saw nothing — which is exactly
        // what happens when the agent committed its own work.
        let decls: &[crate::domain::artifact::ArtifactDecl] =
            step_conf.artifacts.as_deref().unwrap_or(&[]);
        let always: Vec<&str> = decls
            .iter()
            .filter_map(|d| match &d.capture {
                crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                    Some(path.as_str())
                }
                _ => None,
            })
            .collect();
        let mut changed = snapshot
            .delta(&*self.exec, machine_str, wt_path, &always, &[])
            .await;
        if changed.is_empty() {
            if let Some(ref base) = pre_head {
                if let Ok(diff_files) = self
                    .exec
                    .run_command(
                        machine_str,
                        &format!(
                            "git -C {} diff --name-only {}",
                            paths::shell_escape_posix(wt_path),
                            paths::shell_escape_posix(base),
                        ),
                    )
                    .await
                {
                    changed = diff_files
                        .lines()
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
        }
        for rel_path in changed {
            let name = std::path::Path::new(&rel_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("artifact")
                .to_string();
            if let Some(content) =
                read_worktree_file(&*self.exec, machine_str, wt_path, &rel_path).await
            {
                produced_artifacts.push(Artifact::tool_write(name, rel_path, content));
            }
        }

        // Post-step diff guard. A no-op for `Implement` capability (the
        // whole worktree is writable by design); it bites when a workflow
        // gives a sequence step a constrained capture.
        if let Ok(reverted) = self
            .git_ops
            .verify_and_revert_out_of_scope_writes(
                self.machine_id_opt.as_deref(),
                wt_path,
                &self.sequence_writable_paths(step_conf),
            )
            .await
        {
            if !reverted.is_empty() {
                self.capture_signal(
                    Some(step_exec.id.0.clone()),
                    crate::domain::memory::SignalKind::Retry,
                    format!(
                        "Task '{}' wrote outside declared artifacts; reverted: {}. \
                         Stay inside the artifacts directory.",
                        task.id,
                        reverted.join(", ")
                    ),
                );
                return Err((
                    format!(
                        "sequence task '{}' wrote outside declared artifacts; reverted: {}",
                        task.id,
                        reverted.join(", ")
                    ),
                    false,
                ));
            }
        }

        // Commit before the next task starts. This is what makes the task
        // ordering meaningful — task N+1's agent opens a worktree whose HEAD
        // already contains task N — and what the single merge at the end of
        // the step carries. A discarded error here would produce an empty
        // branch and a step that reports success having landed nothing.
        commit_worktree_changes(
            &*self.exec,
            machine_str,
            wt_path,
            &format!(
                "feat({}): {}",
                self.f_id.as_str(),
                task.title.to_lowercase()
            ),
            &self.artifact_subdir,
            self.commit_artifacts,
            &[],
        )
        .await
        .map_err(|e| {
            (
                format!(
                    "sequence task '{}': could not commit the agent's changes, so the task \
                     produced nothing to merge: {}",
                    task.id, e
                ),
                false,
            )
        })?;

        // A declared deliverable missing from *this* task is not a failure —
        // only one task in the list may be the one that writes the report. So
        // record which declarations this task satisfied and let the caller
        // judge the step as a whole, once every task has run.
        let (refs, missing) = resolve_declared_artifacts(
            decls,
            &produced_artifacts,
            &self.artifacts,
            &self.f_id_str,
            &step_exec.step_id.0,
        );
        let missing_names: std::collections::HashSet<&str> =
            missing.iter().map(|m| m.name.as_str()).collect();
        for decl in decls {
            if !missing_names.contains(decl.name.as_str()) {
                satisfied_decls.insert(decl.name.clone());
            }
        }
        all_artifact_refs.extend(refs);

        Ok(())
    }

    /// Spawn a session in the step's worktree.
    ///
    /// Every session a sequence step opens — one per task, plus the one that
    /// resolves a conflicting final merge — is short-lived, keyed to a unique
    /// `thread_id` so the runtime can never hand back a cached session still
    /// carrying an earlier task's conversation, and killed by its caller.
    ///
    /// `Err((message, environmental))`; a spawn failure is always
    /// environmental, a cancellation never is.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_sequence_session(
        &self,
        thread_id: &str,
        title: &str,
        machine_str: &str,
        wt_path: &str,
        agent_kind: &str,
        override_model: &Option<String>,
        effort: crate::domain::models::EffortLevel,
    ) -> Result<std::sync::Arc<dyn crate::ports::agent_runtime::AgentSession>, (String, bool)> {
        let env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;
        let binary = self
            .registry
            .runtime_for(agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.to_string());
        let ctx = AgentContext {
            thread_id: thread_id.to_string(),
            machine_id: machine_str.to_string(),
            binary,
            args: vec![],
            env,
            cwd: wt_path.to_string(),
            model: override_model.clone(),
            // A task turn is real agent work: it inherits the step's effort.
            effort: Some(effort),
            title: Some(title.to_string()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: agent_kind == "claude-code",
            tool_allowlist: None,
            max_turns: None,
            // A sequence task is a primary coding turn: full base budget.
            max_budget_usd: self.role_max_budget_usd(1.0),
        };

        let mut cancel_watch = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = self.registry.get_or_spawn(thread_id, agent_kind, ctx) => Some(res),
            _ = cancel_watch.changed() => None,
        };
        match spawn_res {
            Some(Ok(s)) => Ok(s),
            Some(Err(e)) => Err((format!("agent spawn failed: {:?}", e), true)),
            None => Err(("Execution cancelled by user".to_string(), false)),
        }
    }

    /// Writable-path set for a sequence step. `Implement` capability yields
    /// the "whole worktree" sentinel, which makes both the chmod fence and
    /// the diff guard no-ops — the same contract the parallel implement step
    /// had, and the right one for a step that legitimately writes across the
    /// tree (new files, generated code, build output).
    pub(crate) fn sequence_writable_paths(
        &self,
        step_conf: &StepConfig,
    ) -> Vec<std::path::PathBuf> {
        crate::adapters::worktree::git_ops::scope::derive_writable_paths(
            step_conf.artifacts.as_ref(),
            &self.extra_writable_paths,
        )
    }

    /// Build one task's prompt: the step's template with the task-scoped
    /// placeholders bound, plus the record of what earlier tasks already
    /// landed.
    #[allow(clippy::too_many_arguments)]
    async fn build_task_prompt(
        &self,
        step_conf: &StepConfig,
        step_index: usize,
        step_execs: &[StepExecution],
        task: &PlannedTask,
        task_idx: usize,
        task_total: usize,
        completed: &[CompletedTask],
        resumes_landed_work: bool,
        machine_str: &str,
        wt_path: &str,
    ) -> String {
        let task_files_str = task.files.join(", ");

        // The fresh session has no memory of the earlier tasks, so spell out
        // what is already on the branch. This is the difference between an
        // agent that builds on the previous task and one that reimplements
        // it (or reports "already done" and writes nothing).
        let completed_str = if completed.is_empty() {
            if resumes_landed_work {
                // A retry: nothing has been re-run yet, but the worktree was
                // cut from a feature branch that already carries the previous
                // attempt. Saying "this is the first task" here would send the
                // agent to reimplement code it is looking at.
                "None yet in this attempt — but the code from the previous attempt is already \
                 committed on this branch. Read it first and revise it in place; do not start \
                 over."
                    .to_string()
            } else {
                "None — this is the first task.".to_string()
            }
        } else {
            let mut lines: Vec<String> = completed
                .iter()
                .map(|c| {
                    if c.files.is_empty() {
                        format!("- [{}] {} (already committed)", c.id, c.title)
                    } else {
                        format!(
                            "- [{}] {} (already committed; touched {})",
                            c.id,
                            c.title,
                            c.files.join(", ")
                        )
                    }
                })
                .collect();
            if resumes_landed_work {
                lines.push(
                    "\nThis is a retry: the tasks above are on the branch from the previous \
                     attempt, and so is an earlier version of the task below. Revise it in place."
                        .to_string(),
                );
            }
            lines.join("\n")
        };

        // A task's `retry_note` (stamped by the targeted-retry selection)
        // beats the step-wide feedback, so a re-run task sees the guidance
        // that actually concerns it.
        let retry_feedback = self
            .retry_ctx
            .as_ref()
            .map(|rc| rc.feedback.clone())
            .unwrap_or_default();
        let effective_feedback = task
            .retry_note
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&retry_feedback);
        let effective_retry_ctx = task
            .retry_note
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .map(
                |note| crate::adapters::step_executor::driver::RetryContext {
                    feedback: note.clone(),
                    iteration: self.retry_ctx.as_ref().map_or(1, |rc| rc.iteration),
                    max: self.retry_ctx.as_ref().map_or(1, |rc| rc.max),
                    failing_tests: self
                        .retry_ctx
                        .as_ref()
                        .map(|rc| rc.failing_tests.clone())
                        .unwrap_or_default(),
                    implicated_files: self
                        .retry_ctx
                        .as_ref()
                        .map(|rc| rc.implicated_files.clone())
                        .unwrap_or_default(),
                    failing_step_id: self
                        .retry_ctx
                        .as_ref()
                        .map(|rc| rc.failing_step_id.clone())
                        .unwrap_or_default(),
                },
            )
            .or_else(|| self.retry_ctx.clone());

        let (iteration, max_iterations) = match &self.retry_ctx {
            Some(rc) => (rc.iteration.to_string(), rc.max.to_string()),
            None => (String::new(), String::new()),
        };
        let retry_section = format_retry_feedback_section(effective_retry_ctx.as_ref());

        let template = step_conf.prompt_template.as_deref().unwrap_or("");
        let test_command = task
            .test_command
            .clone()
            .unwrap_or_else(|| self.base_ctx.get("test_command").to_string());

        // The task's own done-definition. A legacy plan (or a trivial task)
        // declares none; say so explicitly rather than rendering an empty
        // section the agent might read as "no obligations".
        let acceptance_str = if task.acceptance.is_empty() {
            "None declared — the task description and the test command define done.".to_string()
        } else {
            task.acceptance
                .iter()
                .map(|c| format!("- {}", c))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let rendered = self
            .base_ctx
            .clone()
            .set("task_id", &task.id)
            .set("task_title", &task.title)
            .set("task_description", &task.description)
            .set("task_files", &task_files_str)
            .set("task_acceptance", &acceptance_str)
            .set("task_index", (task_idx + 1).to_string())
            .set("task_total", task_total.to_string())
            .set("completed_tasks", &completed_str)
            .set("test_command", &test_command)
            // Legacy aliases: a workflow still carrying the old `parallel`
            // prompt (which we now dispatch here) references these names.
            // `other_subtask_files` intentionally renders empty — under
            // sequential execution there is no "files another worker owns,
            // do not touch" set; later tasks may build on earlier ones.
            .set("subtask_description", &task.description)
            .set("subtask_files", &task_files_str)
            .set("other_subtask_files", "")
            .set("partition_id", &task.id)
            .set("retry_feedback_section", &retry_section)
            .set("retry_feedback", effective_feedback)
            .set("iteration", &iteration)
            .set("max_iterations", &max_iterations)
            .render(template);

        let prompt = if rendered.trim().is_empty() {
            format!(
                "Task {}/{}: {}. {}\nFiles: {}\nCode is in: {}\n\nAlready completed:\n{}",
                task_idx + 1,
                task_total,
                task.title,
                task.description,
                task_files_str,
                wt_path,
                completed_str,
            )
        } else {
            resolve_attached_artifacts(
                &rendered,
                step_execs,
                step_index,
                &*self.artifacts,
                &self.steps,
            )
        };

        let prompt = inject_artifact_contract(&prompt, step_conf.artifacts.as_deref());
        let prompt = if template_uses_retry_section(template) {
            prompt
        } else {
            append_retry_feedback_section(prompt, effective_retry_ctx.as_ref())
        };

        crate::adapters::step_executor::artifacts::materialize_external_artifact_paths(
            &prompt,
            wt_path,
            &*self.exec,
            machine_str,
        )
        .await
    }
}
