use std::time::Instant;

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::ids::GateDecisionId;
use crate::domain::models::{GateDecision, StepConfig, StepExecution};
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;
use crate::paths;

impl ExecutionDriver {
    pub(crate) async fn handle_gate_step(
        &self,
        step_exec: &StepExecution,
        _step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        // Use the passed-in step_execs to get the previous step's artifact
        // (avoids an extra DB call — the caller already fetched the list).
        let prev_artifact: Option<String> = if step_index > 0 {
            step_execs.get(step_index - 1).and_then(|s| s.artifact_path.clone())
        } else {
            None
        };

        // Mark gate awaiting decision
        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
            status: Some("awaiting_gate".to_string()),
            cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
            wall_clock_secs: Some(wall).map(|v| Some(v)),
            artifact_path: prev_artifact.as_ref().map(|p| Some(p.clone())),
            error_message: None,
        });
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "awaiting_gate".into(),
            cost_usd: Some(*accumulated_cost),
            wall_clock_secs: Some(wall),
        });

        // Insert GateDecision record
        let gate_dec_id = GateDecisionId::from(format!("gd-{}", step_exec.id.0));
        let gate_dec = GateDecision {
            id: gate_dec_id,
            step_execution_id: step_exec.id.clone(),
            decision: None,
            feedback: None,
            created_at: paths::now_ms(),
        };
        let _ = self.gates.create(gate_dec);
        let _ = self.notif.emit(&DomainEvent::GateRequired {
            feature_id: self.f_id.clone(),
            step_execution_id: step_exec.id.clone(),
        });

        // Set up channel and wait
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<GateDecision>();
        self.gate_senders.lock().unwrap().insert(step_exec.id.0.clone(), gate_tx);

        let mut cancel_watch_gate = self.cancel_watch.clone();
        let gate_res = tokio::select! {
            res = gate_rx => Some(res),
            _ = cancel_watch_gate.changed() => {
                if *cancel_watch_gate.borrow() { None } else { None }
            }
        };

        match gate_res {
            Some(Ok(decision_recvd)) => {
                match decision_recvd.decision.as_deref() {
                    Some("approve") => {
                        let wall = step_start.elapsed().as_secs();
                        let _ = self.features.step_update(&step_exec.id, &StepExecutionPatch {
                            status: Some("completed".to_string()),
                            cost_usd: Some(*accumulated_cost).map(|v| Some(v)),
                            wall_clock_secs: Some(wall).map(|v| Some(v)),
                            artifact_path: prev_artifact.as_ref().map(|p| Some(p.clone())),
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
                    Some("cancel") => {
                        StepOutcome::Failed("Gate Cancelled".to_string())
                    }
                    Some("redirect") => {
                        let target = decision_recvd.feedback.clone()
                            .and_then(|s| if s.is_empty() { None } else { Some(s) })
                            .unwrap_or_else(|| _step_conf.on_failure.as_ref().map(|id| id.0.clone()).unwrap_or_default());
                        if let Some(target_idx) = self.steps.iter().position(|s| s.id.0 == target) {
                            StepOutcome::RedirectTo(target_idx)
                        } else {
                            StepOutcome::Cancelled
                        }
                    }
                    _ => StepOutcome::Cancelled,
                }
            }
            Some(Err(_)) | None => {
                // Cancelled
                let _ = self.gate_senders.lock().unwrap().remove(&step_exec.id.0);
                StepOutcome::Cancelled
            }
        }
    }
}
