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
            machine_id: machine_str,
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
                    let _ = session.set_config_option("model", model);
                }
                let mut artifact_content = String::new();
                let mut stream = session.prompt(&prompt);
                while let Some(event) = stream.next().await {
                    if *self.cancel_watch.borrow() {
                        let _ = session.cancel();
                        break;
                    }
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
                            return StepOutcome::Failed(message.to_string());
                        }
                        _ => {}
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
            Some(Err(e)) => StepOutcome::Failed(e.to_string()),
            None => StepOutcome::Cancelled,
        }
    }
}
