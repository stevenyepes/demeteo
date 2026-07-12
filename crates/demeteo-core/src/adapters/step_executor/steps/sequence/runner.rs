use std::time::Instant;

use super::tasks::PlannedTask;
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

impl ExecutionDriver {
    /// Run `tasks` strictly in order inside the single worktree `wt_path`.
    ///
    /// Every task gets a brand-new agent session (so no context accumulates
    /// across the feature) but the *same* worktree and branch (so each task
    /// builds on the last, and there is nothing to merge between them). A
    /// task commits before the next one starts; the caller merges the whole
    /// branch back once, after this returns.
    ///
    /// `Err((message, environmental))` on the first task that fails — the
    /// caller rolls the feature branch back to its pre-step tip.
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
        tasks: &[PlannedTask],
        machine_str: &str,
        wt_path: &str,
        agent_kind: &str,
        override_model: &Option<String>,
        all_artifact_refs: &mut Vec<String>,
    ) -> Result<(), (String, bool)> {
        let mut completed: Vec<CompletedTask> = Vec::new();

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
            self.run_one_task(
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
                machine_str,
                wt_path,
                agent_kind,
                override_model,
                all_artifact_refs,
            )
            .await?;

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
        machine_str: &str,
        wt_path: &str,
        agent_kind: &str,
        override_model: &Option<String>,
        all_artifact_refs: &mut Vec<String>,
    ) -> Result<(), (String, bool)> {
        // A session per task, not per step: the thread id carries the task
        // id so the runtime can never hand us a cached session still
        // holding the previous task's conversation.
        let thread_id = format!("{}-{}-{}", self.f_id_str, step_exec.step_id.0, task.id);

        let snapshot = WorktreeSnapshot::capture(&*self.exec, machine_str, wt_path).await;

        let prompt = self
            .build_task_prompt(
                step_conf,
                step_index,
                step_execs,
                task,
                task_idx,
                task_total,
                completed,
                machine_str,
                wt_path,
            )
            .await;

        let env =
            crate::ports::agent_runtime::agent_base_env(self.exec.as_ref(), machine_str).await;
        let binary = self
            .registry
            .runtime_for(agent_kind)
            .map(|r| r.binary().to_string())
            .unwrap_or_else(|| agent_kind.to_string());
        let ctx = AgentContext {
            thread_id: thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary,
            args: vec![],
            env,
            cwd: wt_path.to_string(),
            model: override_model.clone(),
            title: Some(task.title.clone()),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            permissions: crate::domain::permission::PermissionProfile::all_allow(),
            bare_mode: agent_kind == "claude-code",
        };

        let mut cancel_watch = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = self.registry.get_or_spawn(&thread_id, agent_kind, ctx) => Some(res),
            _ = cancel_watch.changed() => None,
        };
        let session = match spawn_res {
            Some(Ok(s)) => s,
            Some(Err(e)) => {
                return Err((
                    format!("sequence task '{}': agent spawn failed: {:?}", task.id, e),
                    true,
                ))
            }
            None => return Err(("Execution cancelled by user".to_string(), false)),
        };

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
        let _ = self.registry.kill(&thread_id).await;

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
        // the branch tip when the snapshot saw nothing (e.g. the agent
        // committed its own work).
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
            if let Ok(diff_files) = self
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} diff --name-only HEAD",
                        paths::shell_escape_posix(wt_path),
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

        let (refs, _missing) = resolve_declared_artifacts(
            decls,
            &produced_artifacts,
            &self.artifacts,
            &self.f_id_str,
            &step_exec.step_id.0,
        );
        all_artifact_refs.extend(refs);

        Ok(())
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
        machine_str: &str,
        wt_path: &str,
    ) -> String {
        let task_files_str = task.files.join(", ");

        // The fresh session has no memory of the earlier tasks, so spell out
        // what is already on the branch. This is the difference between an
        // agent that builds on the previous task and one that reimplements
        // it (or reports "already done" and writes nothing).
        let completed_str = if completed.is_empty() {
            "None — this is the first task.".to_string()
        } else {
            completed
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
                .collect::<Vec<_>>()
                .join("\n")
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

        let rendered = self
            .base_ctx
            .clone()
            .set("task_id", &task.id)
            .set("task_title", &task.title)
            .set("task_description", &task.description)
            .set("task_files", &task_files_str)
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
