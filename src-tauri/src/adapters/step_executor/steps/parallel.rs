use std::time::Instant;

use tokio_stream::StreamExt;

use crate::adapters::step_executor::artifacts::resolve_attached_artifacts;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

impl ExecutionDriver {
    pub(crate) async fn handle_parallel_step(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        let feature = self.features.get(&self.f_id).ok().flatten();
        let override_agent = feature.as_ref().and_then(|f| f.agent_kind.clone());
        let override_model = feature.as_ref().and_then(|f| f.model.clone());

        let subtasks = vec![
            ("sub-1", "Implement core logic"),
            ("sub-2", "Write unit tests"),
        ];

        let mut subtask_artifacts = Vec::new();
        let mut step_failed = false;
        let mut step_err_msg = String::new();

        for (sub_idx, (sub_id, sub_title)) in subtasks.iter().enumerate() {
            if *self.cancel_watch.borrow() {
                step_failed = true;
                step_err_msg = "Execution cancelled by user".to_string();
                break;
            }

            // Provision subtask worktree
            let wt_path = match self.git_ops.provision_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                &self.branch_name,
                sub_id,
            ) {
                Ok(p) => p,
                Err(e) => {
                    step_failed = true;
                    step_err_msg =
                        format!("parallel subtask worktree provision failed ({}): {}", sub_id, e);
                    break;
                }
            };

            let agent_kind = override_agent
                .clone()
                .or_else(|| step_conf.agent_kind.clone())
                .unwrap_or_else(|| "opencode".to_string());

            let subtask_files_str =
                "All relevant files for this subtask (feel free to create or edit files as needed to implement the logic)"
                    .to_string();
            let other_files_str = subtasks
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != sub_idx)
                .map(|(_, (_, t))| &**t)
                .collect::<Vec<&str>>()
                .join(", ");

            let sub_prompt = self
                .base_ctx
                .clone()
                .set("subtask_description", &**sub_title)
                .set("subtask_files", &subtask_files_str)
                .set("other_subtask_files", &other_files_str)
                .set("partition_id", &**sub_id)
                .render(step_conf.prompt_template.as_deref().unwrap_or(""));

            let sub_prompt = if sub_prompt.trim().is_empty() {
                format!("Task: {}. Code inside: {}", sub_title, wt_path)
            } else {
                resolve_attached_artifacts(&sub_prompt, step_execs, step_index)
            };

            let machine_str = self
                .machine_id_opt
                .clone()
                .unwrap_or_else(|| "local".to_string());
            let ctx = AgentContext {
                thread_id: format!("{}-{}", self.f_id_str, sub_id),
                machine_id: machine_str.clone(),
                binary: agent_kind.clone(),
                args: vec!["acp".to_string()],
                env: std::collections::HashMap::new(),
                cwd: wt_path.clone(),
                agent_exec: self.agent_exec.clone(),
                exec: self.exec.clone(),
            };

            let sub_thread_id = format!("{}-{}", self.f_id_str, sub_id);
            let spawn_fut =
                self.registry
                    .get_or_spawn(&sub_thread_id, &agent_kind, ctx);
            let mut cancel_watch_spawn = self.cancel_watch.clone();
            let spawn_res = tokio::select! {
                res = spawn_fut => Some(res),
                _ = cancel_watch_spawn.changed() => {
                    if *cancel_watch_spawn.borrow() { None } else { None }
                }
            };

            match spawn_res {
                Some(Ok(session)) => {
                    if let Some(ref model) = override_model {
                        match session.set_config_option("model", model) {
                            Ok(_) => println!("[parallel step] set_config_option model to '{}' succeeded", model),
                            Err(e) => eprintln!("[parallel step] set_config_option model to '{}' failed: {}", model, e),
                        }
                    }
                    let mut stream = session.prompt(&sub_prompt);
                    let mut sub_content = String::new();
                    let mut cancel_watch = self.cancel_watch.clone();
                    let timeout_dur = std::time::Duration::from_secs(180);
                    let sleep_fut = tokio::time::sleep(timeout_dur);
                    tokio::pin!(sleep_fut);

                    loop {
                        tokio::select! {
                            event_opt = stream.next() => {
                                let event = match event_opt {
                                    Some(ev) => ev,
                                    None => break,
                                };

                                // Reset the timeout timer whenever we receive ANY event from the agent
                                sleep_fut.as_mut().reset(tokio::time::Instant::now() + timeout_dur);

                                match event {
                                    AgentEvent::Text { delta } => {
                                        sub_content.push_str(&delta);
                                        let _ = self.notif.emit(&DomainEvent::AgentStream {
                                            feature_id: self.f_id.clone(),
                                            step_execution_id: step_exec.id.clone(),
                                            content: delta.clone(),
                                        });
                                    }
                                    AgentEvent::Usage { cost_usd, .. } => {
                                        if let Some(c) = cost_usd {
                                            *accumulated_cost += c;
                                        }
                                    }
                                    AgentEvent::Error { message, .. } => {
                                        step_failed = true;
                                        let descriptive = super::agent::format_agent_error_message(&message, &machine_str, &*self.exec);
                                        step_err_msg =
                                            format!("parallel subtask agent error ({}): {}", sub_id, descriptive);
                                        break;
                                    }
                                    AgentEvent::TurnComplete { .. } => break,
                                    _ => {}
                                }
                            }
                            _ = &mut sleep_fut => {
                                eprintln!("[parallel step] Subtask {} agent silent timeout of 180s reached.", sub_id);
                                step_failed = true;
                                let descriptive = super::agent::format_agent_error_message("agent response timed out (no output for 180s)", &machine_str, &*self.exec);
                                step_err_msg = format!("parallel subtask agent response timed out ({}) - details: {}", sub_id, descriptive);
                                break;
                            }
                            _ = cancel_watch.changed() => {
                                if *cancel_watch.borrow() {
                                    let _ = session.cancel();
                                    step_failed = true;
                                    step_err_msg = "Execution cancelled by user".to_string();
                                    break;
                                }
                            }
                        }
                    }

                    if step_failed {
                        let _ = self.registry.kill(&sub_thread_id).await;
                        let _ =
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = self.git_ops.cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            sub_id,
                        );
                        break;
                    }

                    subtask_artifacts.push(format!("### {}\n\n{}", sub_title, sub_content));

                    // Merge back
                    if let Err(e) = self.git_ops.merge_subtask(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        &self.branch_name,
                        sub_id,
                    ) {
                        step_failed = true;
                        step_err_msg =
                            format!("parallel subtask merge failed ({}): {}", sub_id, e);
                        let _ = self.registry.kill(&sub_thread_id).await;
                        let _ =
                            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                        let _ = self.git_ops.cleanup_subtask_worktree(
                            self.machine_id_opt.as_deref(),
                            &self.target_dir,
                            sub_id,
                        );
                        break;
                    }
                }
                Some(Err(e)) => {
                    step_failed = true;
                    step_err_msg = format!(
                        "parallel subtask agent spawn failed ({}): {:?}",
                        sub_id, e
                    );
                    let _ = self.registry.kill(&sub_thread_id).await;
                    let _ =
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let _ = self.git_ops.cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        sub_id,
                    );
                    break;
                }
                None => {
                    step_failed = true;
                    step_err_msg = "Execution cancelled by user".to_string();
                    let _ = self.registry.kill(&sub_thread_id).await;
                    let _ =
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    let _ = self.git_ops.cleanup_subtask_worktree(
                        self.machine_id_opt.as_deref(),
                        &self.target_dir,
                        sub_id,
                    );
                    break;
                }
            }

            // Cleanup worktree (success path)
            let _ = self.registry.kill(&sub_thread_id).await;
            let _ = tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            let _ = self.git_ops.cleanup_subtask_worktree(
                self.machine_id_opt.as_deref(),
                &self.target_dir,
                sub_id,
            );
        }

        if step_failed {
            let is_cancelled = *self.cancel_watch.borrow();
            let status_str = if is_cancelled { "interrupted" } else { "failed" };
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
                status: Some(status_str.to_string()),
                cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                wall_clock_secs: Some(wall).map(|v| Some(v)),
                artifact_path: None,
                error_message: Some(Some(step_err_msg.clone())),
            });
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: status_str.into(),
                cost_usd: Some(*accumulated_cost),
                wall_clock_secs: Some(wall),
            });
            if is_cancelled {
                return StepOutcome::Cancelled;
            }
            return StepOutcome::Failed(step_err_msg);
        }

        // Write parallel step artifact summary
        let mut art_path = self.app_local_data_dir.join("artifacts").join(&self.f_id_str);
        let _ = std::fs::create_dir_all(&art_path);
        let file_name = format!("{}.md", step_exec.step_id.0);
        art_path.push(&file_name);
        let _ = std::fs::write(&art_path, subtask_artifacts.join("\n\n"));
        let art_path_str = art_path.to_string_lossy().to_string();

        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
            status: Some("completed".to_string()),
            cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
            wall_clock_secs: Some(wall).map(|v| Some(v)),
            artifact_path: Some(art_path_str).map(|v| Some(v)),
            error_message: None,
        });
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "completed".into(),
            cost_usd: Some(*accumulated_cost),
            wall_clock_secs: Some(wall),
        });
        StepOutcome::Completed
    }
}
