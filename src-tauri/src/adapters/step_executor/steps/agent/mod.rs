use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

pub(crate) mod artifacts;
pub(crate) mod error_message;
pub(crate) mod spawn;

pub(crate) use error_message::format_agent_error_message;

impl ExecutionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn handle_agent_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        accumulated_tokens: &mut i64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        let (agent_kind, override_model) = self.resolve_step_agent(step_conf);

        let (gate_decision, gate_feedback) =
            crate::adapters::step_executor::artifacts::get_latest_gate_decision(
                &*self.gates,
                self.f_id.as_str(),
            );

        let (retry_feedback, retry_iteration, retry_max) = match &self.retry_ctx {
            Some(rc) => (
                rc.feedback.clone(),
                rc.iteration.to_string(),
                rc.max.to_string(),
            ),
            None => (String::new(), String::new(), String::new()),
        };

        let prompt = self
            .base_ctx
            .clone()
            .set("gate_feedback", &gate_feedback)
            .set("gate_decision", &gate_decision)
            .set("retry_feedback", &retry_feedback)
            .set("iteration", &retry_iteration)
            .set("max_iterations", &retry_max)
            .render(step_conf.prompt_template.as_deref().unwrap_or(""));
        let prompt = crate::adapters::step_executor::artifacts::resolve_attached_artifacts(
            &prompt,
            step_execs,
            step_index,
            &*self.artifacts,
        );

        let is_legacy = step_conf.artifacts.as_ref().is_none_or(|d| d.is_empty());
        let decls = step_conf.artifacts.as_deref().unwrap_or(&[]);
        let prompt = crate::adapters::step_executor::artifacts::inject_artifact_contract(
            &prompt,
            if is_legacy { None } else { Some(decls) },
        );

        let machine_str = self
            .machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

        let subtask_id = format!("step-{}", step_exec.step_id.0);
        let wt_path = match self
            .git_ops
            .provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            )
            .await
        {
            Ok(p) => p,
            Err(e) => {
                return StepOutcome::Failed(format!(
                    "agent step worktree provision failed ({}): {}",
                    subtask_id, e
                ));
            }
        };

        if *self.cancel_watch.borrow() {
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            return StepOutcome::Cancelled;
        }

        // Snapshot worktree before running
        let worktree_snapshot =
            crate::adapters::step_executor::artifacts::WorktreeSnapshot::capture(
                &*self.exec,
                &machine_str,
                &wt_path,
            )
            .await;

        let worktree_base_ref = self
            .exec
            .run_command(
                &machine_str,
                &format!(
                    "git -C {} rev-parse {}",
                    crate::paths::shell_escape_posix(&self.target_dir),
                    crate::paths::shell_escape_posix(&self.branch_name),
                ),
            )
            .await
            .map(|s| s.trim().to_string())
            .ok();

        // 1. Spawn session
        let session = match self
            .spawn_agent_session(
                step_exec,
                step_conf,
                &agent_kind,
                &override_model,
                &machine_str,
                &wt_path,
            )
            .await
        {
            Ok(s) => s,
            Err(e) => {
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
                    .await;
                let descriptive =
                    error_message::format_agent_error_message(&e, &machine_str, &*self.exec).await;
                return StepOutcome::Failed(descriptive);
            }
        };

        // 2. Stream turn
        let mut run_failed = None;
        let mut run_cancelled = false;
        let timeouts = crate::adapters::agent::event_stream::Timeouts {
            fast_timeout_s: 180,
            normal_timeout_s: 180,
            wall_cap_s: 600,
        };

        let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
            &*session,
            &prompt,
            timeouts,
            Some(self.cancel_watch.clone()),
            &machine_str,
            &*self.exec,
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
                        cost_usd: Some(*accumulated_cost),
                        tokens: Some(*accumulated_tokens),
                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                    });
                }
            },
        )
        .await;

        let mut produced_artifacts = Vec::new();
        let mut text_buffer = String::new();

        match turn_res {
            crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                run_cancelled = true;
            }
            crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                run_failed = Some(StepOutcome::Failed(descriptive));
            }
            crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                *accumulated_cost += outcome.cost_usd;
                *accumulated_tokens += outcome.tokens;
                produced_artifacts = outcome.produced_artifacts;
                text_buffer = outcome.text;
            }
        }

        if run_cancelled || *self.cancel_watch.borrow() {
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "interrupted".into(),
                cost_usd: Some(*accumulated_cost),
                tokens: Some(*accumulated_tokens),
                wall_clock_secs: Some(wall),
            });
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            let _ = self.registry.kill(self.f_id.as_str()).await;
            return StepOutcome::Cancelled;
        }

        if let Some(failed_outcome) = run_failed {
            let _ = self
                .git_ops
                .cleanup_subtask_worktree(
                    self.machine_id_opt.as_deref(),
                    &self.target_dir,
                    &self.branch_name,
                    &subtask_id,
                )
                .await;
            let _ = self.registry.kill(self.f_id.as_str()).await;
            return failed_outcome;
        }

        // 3. Process artifacts (delta, diff, commit, resolve decls)
        let artifacts_res = self
            .process_agent_artifacts(
                step_exec,
                step_conf,
                &machine_str,
                &wt_path,
                &worktree_snapshot,
                &worktree_base_ref,
                &mut produced_artifacts,
            )
            .await;

        let (mut artifact_path, mut artifact_paths) = match artifacts_res {
            Ok((path, paths)) => (path, paths),
            Err(err) => {
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
                    .await;
                let _ = self.registry.kill(self.f_id.as_str()).await;
                return StepOutcome::Failed(err);
            }
        };

        // 4. Run verifier
        if let Some(ref verifier_cfg) = step_conf.verifier {
            if let Err(verdict_err) = self
                .run_verifier_logic(
                    step_exec,
                    verifier_cfg,
                    &wt_path,
                    &produced_artifacts,
                    accumulated_cost,
                    accumulated_tokens,
                    step_start,
                    &agent_kind,
                    &override_model,
                    &machine_str,
                )
                .await
            {
                let _ = self
                    .registry
                    .kill(&format!("{}-verifier", self.f_id.as_str()))
                    .await;
                let _ = self
                    .git_ops
                    .cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        &subtask_id,
                    )
                    .await;
                let _ = self.registry.kill(self.f_id.as_str()).await;
                return StepOutcome::Failed(verdict_err);
            }
        }

        // 5. Merge subtask back
        let mut merge_result = self
            .git_ops
            .merge_subtask(
                self.machine_id_opt.as_deref(),
                &wt_path,
                &self.branch_name,
                &subtask_id,
            )
            .await;

        // Resolve conflicts if merge failed
        if let Err(ref e) = merge_result {
            let merge_back_cmd = format!(
                "git -C {} merge {}",
                crate::paths::shell_escape_posix(&wt_path),
                crate::paths::shell_escape_posix(&self.branch_name)
            );
            let _ = self.exec.run_command(&machine_str, &merge_back_cmd).await;

            let unmerged =
                crate::adapters::step_executor::steps::parallel::list_unmerged::list_unmerged_files(&*self.exec, &machine_str, &wt_path).await;
            if !unmerged.is_empty() {
                let files_list = unmerged
                    .iter()
                    .map(|f| format!("- {} ({})", f.path, f.kind))
                    .collect::<Vec<_>>()
                    .join("\n");
                let conflict_prompt = format!(
                    "We encountered a merge conflict while merging the latest changes from the feature branch '{}' into your workspace.\n\
                     Please resolve the conflicts in the following files:\n\
                     {}\n\n\
                     Ensure you edit these files to remove conflict markers (<<<<<<<, =======, >>>>>>>) and integrate the changes correctly. \
                     Make sure all code builds and passes tests. Once done, let me know.",
                    self.branch_name, files_list
                );

                let turn_res = crate::adapters::agent::event_stream::stream_agent_turn(
                    &*session,
                    &conflict_prompt,
                    timeouts,
                    Some(self.cancel_watch.clone()),
                    &machine_str,
                    &*self.exec,
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
                                cost_usd: Some(*accumulated_cost),
                                tokens: Some(*accumulated_tokens),
                                wall_clock_secs: Some(step_start.elapsed().as_secs()),
                            });
                        }
                    },
                )
                .await;

                let mut conflict_failed = None;
                let mut conflict_cancelled = false;

                match turn_res {
                    crate::adapters::agent::event_stream::TurnResult::Interrupted => {
                        conflict_cancelled = true;
                    }
                    crate::adapters::agent::event_stream::TurnResult::Failed(descriptive) => {
                        conflict_failed = Some(StepOutcome::Failed(descriptive));
                    }
                    crate::adapters::agent::event_stream::TurnResult::Success(outcome) => {
                        *accumulated_cost += outcome.cost_usd;
                        *accumulated_tokens += outcome.tokens;
                    }
                }

                if conflict_cancelled || *self.cancel_watch.borrow() {
                    run_cancelled = true;
                } else if let Some(failed_outcome) = conflict_failed {
                    run_failed = Some(failed_outcome);
                } else {
                    // Verify conflicts are resolved.
                    let still_unmerged =
                        crate::adapters::step_executor::steps::parallel::list_unmerged::list_unmerged_files(&*self.exec, &machine_str, &wt_path).await;
                    if still_unmerged.is_empty() {
                        let commit_resolved = self
                            .exec
                            .run_command(
                                &machine_str,
                                &format!(
                                    "git -C {} commit -am \"Resolve merge conflicts with {}\"",
                                    crate::paths::shell_escape_posix(&wt_path),
                                    crate::paths::shell_escape_posix(&self.branch_name)
                                ),
                            )
                            .await;
                        if commit_resolved.is_ok() {
                            merge_result = self
                                .git_ops
                                .merge_subtask(
                                    self.machine_id_opt.as_deref(),
                                    &wt_path,
                                    &self.branch_name,
                                    &subtask_id,
                                )
                                .await;
                        } else {
                            merge_result =
                                Err("Failed to commit merge conflict resolution".to_string());
                        }
                    } else {
                        merge_result = Err(format!(
                            "Agent failed to resolve merge conflicts in: {:?}",
                            still_unmerged.iter().map(|f| &f.path).collect::<Vec<_>>()
                        ));
                    }
                }
            } else {
                merge_result = Err(format!("agent step merge failed: {}", e));
            }
        }

        let outcome = if run_cancelled || *self.cancel_watch.borrow() {
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some("interrupted".to_string()),
                    cost_usd: Some(Some(*accumulated_cost)),
                    tokens: Some(Some(*accumulated_tokens)),
                    wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some("Execution cancelled by user".to_string())),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "interrupted".into(),
                cost_usd: Some(*accumulated_cost),
                tokens: Some(*accumulated_tokens),
                wall_clock_secs: Some(wall),
            });
            StepOutcome::Cancelled
        } else if let Some(failed_outcome) = run_failed {
            failed_outcome
        } else {
            match merge_result {
                Ok(()) => {
                    // Resolve artifacts: either declared (new) or text-dump (legacy)
                    if is_legacy {
                        let mut art_path = self
                            .app_local_data_dir
                            .join("artifacts")
                            .join(&self.f_id_str);
                        let _ = std::fs::create_dir_all(&art_path);
                        let file_name = format!("{}.md", step_exec.step_id.0);
                        art_path.push(&file_name);
                        let _ = std::fs::write(&art_path, &text_buffer);
                        let art_path_str = art_path.to_string_lossy().to_string();
                        artifact_path = Some(art_path_str.clone());
                        artifact_paths = vec![art_path_str];
                    }

                    let wall = step_start.elapsed().as_secs();
                    let _ = self.features.step_update(
                        &step_exec.id,
                        &StepExecutionPatch {
                            iteration_count: None,
                            status: Some("completed".to_string()),
                            cost_usd: Some(Some(*accumulated_cost)),
                            tokens: Some(Some(*accumulated_tokens)),
                            wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                            artifact_path: Some(artifact_path),
                            artifact_paths: Some(artifact_paths),
                            error_message: Some(None),
                        },
                    );
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "completed".into(),
                        cost_usd: Some(*accumulated_cost),
                        tokens: Some(*accumulated_tokens),
                        wall_clock_secs: Some(wall),
                    });
                    StepOutcome::Completed
                }
                Err(err) => StepOutcome::Failed(format!("agent step merge failed: {}", err)),
            }
        };

        // Cleanup temporary worktree in all cases.
        let _ = self
            .git_ops
            .cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                &subtask_id,
            )
            .await;

        let _ = self.registry.kill(self.f_id.as_str()).await;
        let _ = self
            .registry
            .kill(&format!("{}-verifier", self.f_id.as_str()))
            .await;

        outcome
    }
}
