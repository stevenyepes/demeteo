use std::time::Instant;

use tokio_stream::StreamExt;

use crate::adapters::step_executor::artifacts::{inject_artifact_contract, resolve_attached_artifacts, resolve_declared_artifacts};
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::artifact::Artifact;
use crate::domain::agent_event::AgentEvent;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::agent_runtime::AgentContext;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

impl ExecutionDriver {
    pub(crate) async fn handle_agent_step(
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

        let agent_kind = override_agent
            .or_else(|| step_conf.agent_kind.clone())
            .unwrap_or_else(|| "opencode".to_string());

        let (gate_decision, gate_feedback) = crate::adapters::step_executor::artifacts::get_latest_gate_decision(
            &*self.gates,
            self.f_id.as_str(),
        );

        let prompt = self.base_ctx.clone()
            .set("gate_feedback", &gate_feedback)
            .set("gate_decision", &gate_decision)
            .render(step_conf.prompt_template.as_deref().unwrap_or(""));
        let prompt = resolve_attached_artifacts(&prompt, step_execs, step_index);

        // Inject the machine-readable artifact contract into the prompt
        // so the agent knows exactly what files to produce.
        let is_legacy = step_conf.artifacts.as_ref().map_or(true, |d| d.is_empty());
        let decls: &[crate::domain::artifact::ArtifactDecl] = step_conf.artifacts.as_deref().unwrap_or(&[]);
        let prompt = inject_artifact_contract(&prompt, if is_legacy { None } else { Some(decls) });

        let machine_str = self.machine_id_opt.clone().unwrap_or_else(|| "local".to_string());
        let mut agent_env = crate::ports::agent_runtime::agent_base_env();
        if let Some(ref m) = override_model {
            let config = format!(
                r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                m
            );
            agent_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
        }
        let ctx = AgentContext {
            thread_id: self.f_id_str.clone(),
            machine_id: machine_str.clone(),
            binary: agent_kind.clone(),
            args: vec!["acp".to_string()],
            env: agent_env,
            cwd: self.target_dir.clone(),
            model: override_model.clone(),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
        };

        if *self.cancel_watch.borrow() {
            return StepOutcome::Cancelled;
        }

        let spawn_fut = self.registry.get_or_spawn(self.f_id.as_str(), &agent_kind, ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => {
                if *cancel_watch_spawn.borrow() { None } else { None }
            }
        };

        match spawn_res {
            Some(Ok(session)) => {
                // Verify the model from session/new — fall back to
                // set_config_option if the agent didn't apply it.
                if let Some(ref model) = override_model {
                    let info = session.session_info();
                    let applied = info.config_options.as_ref().and_then(|opts|
                        opts.iter().find(|o| o.id == "model")
                    ).map(|o| o.current_value == *model).unwrap_or(false);
                    if !applied {
                        eprintln!(
                            "[agent step] model '{}' not applied in session/new (current={:?}), trying set_config_option",
                            model,
                            info.config_options.as_ref().and_then(|opts|
                                opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                            )
                        );
                        let config_ok = match session.set_config_option("model", model) {
                            Ok(_) => {
                                let info2 = session.session_info();
                                let really_applied = info2.config_options.as_ref().and_then(|opts|
                                    opts.iter().find(|o| o.id == "model")
                                ).map(|o| o.current_value == *model).unwrap_or(false);
                                if really_applied {
                                    println!("[agent step] set_config_option model to '{}' confirmed in session_info", model);
                                    true
                                } else {
                                    eprintln!(
                                        "[agent step] set_config_option returned Ok but model '{}' STILL not applied (current={:?})",
                                        model,
                                        info2.config_options.as_ref().and_then(|opts|
                                            opts.iter().find(|o| o.id == "model").map(|o| &o.current_value)
                                        )
                                    );
                                    false
                                }
                            }
                            Err(e) => {
                                eprintln!("[agent step] set_config_option model to '{}' failed: {}", model, e);
                                false
                            }
                        };
                        if !config_ok {
                            eprintln!(
                                 "[agent step] WARNING: model '{}' was NOT applied. Agent will use its default model. \
                                  If using opencode, ensure OPENCODE_CONFIG_CONTENT env var is being picked up.",
                                model
                            );
                        }
                    } else {
                        println!("[agent step] model '{}' confirmed in session_info after spawn", model);
                    }
                }
                // Collect ArtifactProduced events emitted by the runtime
                // during the agent turn. In legacy mode (no declarations)
                // we fall back to the old text-dump approach.
                let mut produced_artifacts: Vec<Artifact> = Vec::new();
                let mut legacy_text_buffer = String::new();
                let mut stream = session.prompt(&prompt);
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
                                    // Always emit stream events for the UI
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
                                        wall_clock_secs: Some(step_start.elapsed().as_secs()),
                                    });
                                    // Legacy mode: accumulate text for the text-dump artifact
                                    if is_legacy {
                                        legacy_text_buffer.push_str(&delta);
                                    }
                                }
                                AgentEvent::ArtifactProduced { artifact } => {
                                    produced_artifacts.push(artifact);
                                }
                                AgentEvent::Usage { cost_usd, .. } => {
                                    if let Some(c) = cost_usd {
                                        *accumulated_cost += c;
                                    }
                                }
                                AgentEvent::TurnComplete { .. } => break,
                                AgentEvent::Error { message, .. } => {
                                    let _ = self.registry.kill(self.f_id.as_str()).await;
                                    let descriptive = format_agent_error_message(&message, &machine_str, &*self.exec);
                                    return StepOutcome::Failed(descriptive);
                                }
                                _ => {}
                            }
                        }
                        _ = &mut sleep_fut => {
                            eprintln!("[agent step] Agent silent timeout of 180s reached.");
                            let _ = self.registry.kill(self.f_id.as_str()).await;
                            let descriptive = format_agent_error_message("Agent response timed out (no output for 180s)", &machine_str, &*self.exec);
                            return StepOutcome::Failed(descriptive);
                        }
                        _ = cancel_watch.changed() => {
                            if *cancel_watch.borrow() {
                                let _ = session.cancel();
                                break;
                            }
                        }
                    }
                }

                if *self.cancel_watch.borrow() {
                    let wall = step_start.elapsed().as_secs();
                    let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
        iteration_count: None,
                        status: Some("interrupted".to_string()),
                        cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                        wall_clock_secs: Some(wall).map(|v| Some(wall)),
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: Some(Some("Execution cancelled by user".to_string())),
                    });
                    let _ = self.notif.emit(&DomainEvent::StepProgress {
                        feature_id: self.f_id.clone(),
                        step_id: step_exec.step_id.0.clone(),
                        status: "interrupted".into(),
                        cost_usd: Some(*accumulated_cost),
                        wall_clock_secs: Some(wall),
                    });
                    let _ = self.registry.kill(self.f_id.as_str()).await;
                    return StepOutcome::Cancelled;
                }

                // Resolve artifacts: either declared (new) or text-dump (legacy)
                let (artifact_path, artifact_paths) = if !is_legacy {
                    let refs = resolve_declared_artifacts(
                        decls,
                        &produced_artifacts,
                        &self.artifacts,
                        &self.f_id_str,
                        &step_exec.step_id.0,
                    );
                    let primary = refs.first().cloned();
                    (primary, refs)
                } else {
                    let mut art_path = self.app_local_data_dir.join("artifacts").join(&self.f_id_str);
                    let _ = std::fs::create_dir_all(&art_path);
                    let file_name = format!("{}.md", step_exec.step_id.0);
                    art_path.push(&file_name);
                    let _ = std::fs::write(&art_path, &legacy_text_buffer);
                    let art_path_str = art_path.to_string_lossy().to_string();
                    (Some(art_path_str.clone()), vec![art_path_str])
                };

                let wall = step_start.elapsed().as_secs();
                let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
        iteration_count: None,
                    status: Some("completed".to_string()),
                    cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                    wall_clock_secs: Some(wall).map(|v| Some(wall)),
                    artifact_path: Some(artifact_path),
                    artifact_paths: Some(artifact_paths),
                    error_message: Some(None),
                });
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: step_exec.step_id.0.clone(),
                    status: "completed".into(),
                    cost_usd: Some(*accumulated_cost),
                    wall_clock_secs: Some(wall),
                });
                let _ = self.registry.kill(self.f_id.as_str()).await;
                StepOutcome::Completed
            }
            Some(Err(e)) => {
                let descriptive = format_agent_error_message(&e.to_string(), &machine_str, &*self.exec);
                StepOutcome::Failed(descriptive)
            }
            None => StepOutcome::Cancelled,
        }
    }
}

pub(crate) fn format_agent_error_message(
    message: &str,
    machine_id: &str,
    exec: &dyn crate::ports::execution::ExecutionPort,
) -> String {
    if message.contains("OpenCode service failure")
        || message.contains("timed out")
        || message.contains("no output")
        || message.is_empty()
    {
        // Fetch last 100 lines of remote log
        if let Ok(logs) = exec.run_command(machine_id, "tail -n 100 /tmp/opencode_run.log") {
            if logs.contains("FreeUsageLimitError") || logs.contains("Rate limit exceeded") {
                return "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.".to_string();
            }
            if logs.contains("CreditLimitError")
                || logs.contains("Insufficient funds")
                || logs.contains("credits limit")
                || logs.contains("insufficient balance")
            {
                return "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota. Please verify your billing/credits on OpenCode or switch to a free model.".to_string();
            }
            // Fallback search through last lines
            for line in logs.lines().rev() {
                if line.contains("FreeUsageLimitError") || line.contains("Rate limit exceeded") {
                    return "OpenCode Rate Limit Exceeded: The free model rate limit was reached. Please try changing the model to a different model (e.g. 'opencode/big-pickle') or try again later.".to_string();
                }
                if line.contains("CreditLimitError") || line.contains("credits limit") {
                    return "OpenCode Credit Limit Exceeded: You have run out of credits or reached your usage quota. Please verify your billing/credits on OpenCode or switch to a free model.".to_string();
                }
            }
        }
    }
    message.to_string()
}
