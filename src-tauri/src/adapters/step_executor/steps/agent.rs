use std::time::Instant;

use tokio_stream::StreamExt;

use crate::adapters::step_executor::artifacts::resolve_attached_artifacts;
use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::models::{StepConfig, StepExecution};
use crate::domain::agent_event::AgentEvent;
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

        let machine_str = self.machine_id_opt.clone().unwrap_or_else(|| "local".to_string());
        let ctx = AgentContext {
            thread_id: self.f_id_str.clone(),
            machine_id: machine_str.clone(),
            binary: agent_kind.clone(),
            args: vec!["acp".to_string()],
            env: std::collections::HashMap::new(),
            cwd: self.target_dir.clone(),
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
                if let Some(ref model) = override_model {
                    match session.set_config_option("model", model) {
                        Ok(_) => println!("[agent step] set_config_option model to '{}' succeeded", model),
                        Err(e) => eprintln!("[agent step] set_config_option model to '{}' failed: {}", model, e),
                    }
                }
                let mut artifact_content = String::new();
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
                                    artifact_content.push_str(&delta);
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
                        status: Some("interrupted".to_string()),
                        cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                        wall_clock_secs: Some(wall).map(|v| Some(v)),
                        artifact_path: None,
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

                // Write artifact to disk
                let mut art_path = self.app_local_data_dir.join("artifacts").join(&self.f_id_str);
                let _ = std::fs::create_dir_all(&art_path);
                let file_name = format!("{}.md", step_exec.step_id.0);
                art_path.push(&file_name);
                let _ = std::fs::write(&art_path, &artifact_content);
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
