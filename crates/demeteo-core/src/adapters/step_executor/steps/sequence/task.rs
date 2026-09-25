//! One task of the list: a fresh session, one turn, the diff guard, and the
//! commit that makes the next task's worktree contain this one's work.

use crate::adapters::step_executor::artifacts::agent_commits::capture_head;
use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, read_worktree_file, resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::artifact_capture::captures_file_bodies;
use crate::domain::sequence::outcome::SequenceError;
use crate::domain::sequence::progress::TaskContribution;
use crate::ports::notification::DomainEvent;

use super::context::{RunTarget, StepCtx, StepSpend, StepWorktree, TaskRun};

impl ExecutionDriver {
    /// One task: fresh session, one turn, diff guard, commit. Never merges
    /// and never touches the feature branch — the caller owns that.
    ///
    /// Returns what the task produced. `spend` stays `&mut` because its
    /// totals are not step-scoped: the driver carries them across every step
    /// of the feature.
    pub(crate) async fn run_one_task(
        &self,
        step: StepCtx<'_>,
        spend: &mut StepSpend<'_>,
        target: RunTarget<'_>,
        wt: StepWorktree<'_>,
        run: TaskRun<'_>,
    ) -> Result<TaskContribution, SequenceError> {
        let step_exec = step.step_exec;
        let step_conf = step.step_conf;
        let task = run.task;

        let decls: &[crate::domain::artifact::ArtifactDecl] =
            step_conf.artifacts.as_deref().unwrap_or(&[]);
        // The snapshot exists only to name the paths whose bodies get read
        // back, so it is gated on the same question the agent step asks — a
        // step declaring only a `Diff` was paying for a whole delta whose
        // every body was then discarded.
        let snapshot = if captures_file_bodies(decls) {
            Some(WorktreeSnapshot::capture(&*self.exec, target.machine, wt.path).await)
        } else {
            None
        };
        let pre_turn_head = capture_head(&*self.exec, target.machine, wt.path).await;

        let prompt = self.build_task_prompt(step, target, wt, run).await;

        // A task turn is the step's own work, so the step's answer applies —
        // the only turn of this stage's three for which that is true.
        let target = RunTarget {
            keep_harness_personalization: crate::domain::turn_role::TurnRole::Step(step_conf)
                .keeps_harness_personalization(),
            ..target
        };
        let session = self
            .spawn_sequence_session(target, wt.path, run.thread_id, &task.title)
            .await
            .map_err(|e| e.with_context(format_args!("sequence task '{}'", task.id)))?;

        let timeouts = crate::application::timeouts::resolve_effective(self.app_settings.as_ref());
        let base_cost = *spend.cost;
        let base_tokens = *spend.tokens;
        let mut observer = crate::domain::sequence::report::CommandObserver::default();
        let mut agent_text = String::new();

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            &*session,
            &prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            target.machine,
            &*self.exec,
            target.override_model.map(str::to_string),
            self.pricing.clone(),
            |event| {
                observer.observe(event);
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
                        wall_clock_secs: Some(spend.start.elapsed().as_secs()),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    });
                }
            },
        )
        .await;

        let mut produced_artifacts: Vec<Artifact> = Vec::new();
        let turn_err: Option<SequenceError> = match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *spend.cost += outcome.cost_usd;
                *spend.tokens += outcome.tokens;
                produced_artifacts = outcome.produced_artifacts;
                agent_text = outcome.text;
                None
            }
            crate::adapters::agent::event_stream::TurnResult::Failed { reason, spent } => {
                *spend.cost += spent.cost_usd;
                *spend.tokens += spent.tokens;
                Some(SequenceError::Failed(format!(
                    "sequence task '{}': agent error: {}",
                    task.id, reason
                )))
            }
            crate::adapters::agent::event_stream::TurnResult::Environmental { reason, spent } => {
                *spend.cost += spent.cost_usd;
                *spend.tokens += spent.tokens;
                Some(SequenceError::Environmental(format!(
                    "sequence task '{}': agent error: {}",
                    task.id, reason
                )))
            }
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                Some(SequenceError::Cancelled)
            }
        };

        // The session's work is done either way — a task never needs its
        // conversation again, and leaving it alive would keep the model's
        // context (and the runtime process) around for the whole step.
        let _ = self.registry.kill(run.thread_id).await;

        if let Some(err) = turn_err {
            return Err(err);
        }

        // The step's worktree must still exist. If it does not, the agent's
        // writes were never committed (that happens below) and are gone —
        // report it rather than capturing an empty delta and moving on. See
        // `WorktreeSnapshot::worktree_is_missing`.
        if WorktreeSnapshot::worktree_is_missing(&*self.exec, target.machine, wt.path).await {
            return Err(SequenceError::Environmental(format!(
                "sequence task '{}': the step's worktree '{}' disappeared while the agent \
                 was running — its uncommitted changes are unrecoverable.",
                task.id, wt.path
            )));
        }

        self.fold_agent_commits_into_turn(
            &step_exec.step_id.0,
            Some(&task.id),
            target.machine,
            wt.path,
            &pre_turn_head,
        )
        .await;

        if let Some(snapshot) = snapshot {
            let always: Vec<&str> = decls
                .iter()
                .filter_map(|d| match &d.capture {
                    crate::domain::artifact::ArtifactCapture::LastWriteTo { path } => {
                        Some(path.as_str())
                    }
                    _ => None,
                })
                .collect();
            let changed = snapshot
                .delta(&*self.exec, target.machine, wt.path, &always, &[])
                .await;
            for rel_path in changed {
                let name = std::path::Path::new(&rel_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("artifact")
                    .to_string();
                if let Some(content) =
                    read_worktree_file(&*self.exec, target.machine, wt.path, &rel_path).await
                {
                    produced_artifacts.push(Artifact::tool_write(name, rel_path, content));
                }
            }
        }

        // Post-step diff guard. A no-op for `Implement` capability (the
        // whole worktree is writable by design); it bites when a workflow
        // gives a sequence step a constrained capture.
        if let Ok(reverted) = self
            .git_ops
            .verify_and_revert_out_of_scope_writes(
                self.machine_id_opt.as_deref(),
                wt.path,
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
                return Err(SequenceError::Failed(format!(
                    "sequence task '{}' wrote outside declared artifacts; reverted: {}",
                    task.id,
                    reverted.join(", ")
                )));
            }
        }

        // Commit before the next task starts. This is what makes the task
        // ordering meaningful — task N+1's agent opens a worktree whose HEAD
        // already contains task N — and what the single merge at the end of
        // the step carries. A discarded error here would produce an empty
        // branch and a step that reports success having landed nothing.
        commit_worktree_changes(
            &*self.exec,
            target.machine,
            wt.path,
            &crate::domain::sequence::tasks::task_commit_message(
                self.f_id.as_str(),
                &task.id,
                &task.title,
            ),
            &self.artifact_subdir,
            self.commit_artifacts,
            &[],
        )
        .await
        .map_err(|e| {
            SequenceError::Failed(format!(
                "sequence task '{}': could not commit the agent's changes, so the task \
                 produced nothing to merge: {}",
                task.id, e
            ))
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
        // Only a task that committed gets a fragment: the report is evidence
        // about work on the branch, not about attempts that were rolled back.
        super::report::record_ticket_report(
            &*self.artifacts,
            &self.f_id_str,
            &step_exec.step_id.0,
            &crate::domain::sequence::report::TicketReport::new(
                &task.id,
                &task.title,
                &agent_text,
                observer.into_commands(),
                crate::paths::now_ms(),
            ),
        );
        Ok(TaskContribution {
            artifact_refs: refs,
            satisfied_decls: decls
                .iter()
                .filter(|d| !missing_names.contains(d.name.as_str()))
                .map(|d| d.name.clone())
                .collect(),
        })
    }
}
