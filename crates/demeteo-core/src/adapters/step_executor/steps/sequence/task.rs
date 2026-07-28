//! One task of the list: a fresh session, one turn, the diff guard, and the
//! commit that makes the next task's worktree contain this one's work.

use std::time::Instant;

use crate::adapters::step_executor::artifacts::{
    commit_worktree_changes, read_worktree_file, resolve_declared_artifacts, WorktreeSnapshot,
};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::agent_event::AgentEvent;
use crate::domain::artifact::Artifact;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::sequence::outcome::SequenceError;
use crate::domain::sequence::progress::TaskContribution;
use crate::domain::sequence::tasks::PlannedTask;
use crate::ports::notification::DomainEvent;

use super::prompt::CompletedTask;

impl ExecutionDriver {
    /// One task: fresh session, one turn, diff guard, commit. Never merges
    /// and never touches the feature branch — the caller owns that.
    ///
    /// Returns what the task produced. `accumulated_cost` and
    /// `accumulated_tokens` stay `&mut` because they are not step-scoped:
    /// the driver carries them across every step of the feature.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_one_task(
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
        override_model: Option<&str>,
    ) -> Result<TaskContribution, SequenceError> {
        let snapshot = WorktreeSnapshot::capture(&*self.exec, machine_str, wt_path).await;
        // The worktree's HEAD *before* the agent runs. The snapshot delta
        // misses work the agent committed itself, and diffing against the
        // worktree's own HEAD afterwards would miss it too — the commit moved
        // HEAD. Pinning the pre-turn commit is what lets the fallback below
        // see it. Diffing against the feature branch instead would over-report
        // here in a way it could not in the old parallel step: this worktree
        // already carries every earlier task's commits.
        let pre_head = self
            .sequence_git(machine_str)
            .rev_parse(wt_path, "HEAD")
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
            .map_err(|e| e.with_context(format_args!("sequence task '{}'", task.id)))?;

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
            override_model.map(str::to_string),
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
        let turn_err: Option<SequenceError> = match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                produced_artifacts = outcome.produced_artifacts;
                None
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                Some(SequenceError::Failed(format!(
                    "sequence task '{}': agent error: {}",
                    task.id, descriptive
                )))
            }
            crate::adapters::agent::event_stream::TurnResult::Environmental(descriptive) => {
                Some(SequenceError::Environmental(format!(
                    "sequence task '{}': agent error: {}",
                    task.id, descriptive
                )))
            }
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                Some(SequenceError::Cancelled)
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
            return Err(SequenceError::Environmental(format!(
                "sequence task '{}': the step's worktree '{}' disappeared while the agent \
                 was running — its uncommitted changes are unrecoverable.",
                task.id, wt_path
            )));
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
                    .sequence_git(machine_str)
                    .diff_name_only(wt_path, base)
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
