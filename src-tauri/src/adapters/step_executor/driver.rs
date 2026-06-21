use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{oneshot, watch};

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{GateDecision, StepConfig, StepExecution};
use crate::domain::prompt_context::PromptContext;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::{FeaturePatch, FeatureRepository, GateRepository, StepExecutionPatch};
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::{DomainEvent, NotificationPort};

/// Holds all shared state for a single feature execution run.
/// Fields are `pub(crate)` so step-handler `impl` blocks in child modules can access them.
pub(crate) struct ExecutionDriver {
    // Repository / service Arcs
    pub features: Arc<dyn FeatureRepository>,
    pub gates: Arc<dyn GateRepository>,
    pub notif: Arc<dyn NotificationPort>,
    pub registry: Arc<AgentRegistry>,
    pub agent_exec: Arc<dyn AgentExecutionPort>,
    pub exec: Arc<dyn ExecutionPort>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub app_local_data_dir: PathBuf,
    pub git_ops: GitOpsHelper,
    pub gate_senders: Arc<Mutex<HashMap<String, oneshot::Sender<GateDecision>>>>,

    // Feature identity
    pub f_id: FeatureId,
    pub f_id_str: String,

    // Pre-computed setup
    pub machine_id_opt: Option<String>,
    pub target_dir: String,
    pub branch_name: String,
    pub base_ctx: PromptContext,
    pub steps: Vec<StepConfig>,

    // Mutable execution state
    pub step_index: usize,
    pub start_time: Instant,
    pub cancel_watch: watch::Receiver<bool>,
}

impl ExecutionDriver {
    /// Run the full execution loop, dispatching each step by kind.
    pub(crate) async fn run(mut self) {
        // Determine starting step_index by finding the first non-completed step
        if let Ok(step_execs) = self.features.steps_for_feature(&self.f_id) {
            for s in &step_execs {
                if s.status == "completed" {
                    self.step_index = s.step_index as usize + 1;
                } else {
                    break;
                }
            }
        }

        loop {
            if *self.cancel_watch.borrow() {
                self.cancel_feature().await;
                return;
            }

            let step_execs = match self.features.steps_for_feature(&self.f_id) {
                Ok(list) => list,
                Err(_) => break,
            };

            if self.step_index >= step_execs.len() {
                break;
            }

            let step_exec = &step_execs[self.step_index];
            let step_conf = match self.steps.iter().find(|s| s.id == step_exec.step_id) {
                Some(sc) => sc,
                None => break,
            };

            // Mark step as running (preserve existing cost / wall_clock from DB)
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some("running".to_string()),
                    cost_usd: step_exec.cost_usd.map(|v| Some(v)),
                    wall_clock_secs: step_exec.wall_clock_secs.map(|v| Some(v)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(None),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "running".into(),
                cost_usd: step_exec.cost_usd,
                wall_clock_secs: step_exec.wall_clock_secs,
            });

            let step_start = Instant::now();
            let mut accumulated_cost = step_exec.cost_usd.unwrap_or(0.0);

            let outcome = match step_conf.kind.as_str() {
                "agent" => {
                    self.handle_agent_step(
                        step_exec,
                        step_conf,
                        &mut accumulated_cost,
                        step_start,
                        self.step_index,
                        &step_execs,
                    )
                    .await
                }
                "gate" => {
                    self.handle_gate_step(
                        step_exec,
                        step_conf,
                        &mut accumulated_cost,
                        step_start,
                        self.step_index,
                        &step_execs,
                    )
                    .await
                }
                "parallel" => {
                    self.handle_parallel_step(
                        step_exec,
                        step_conf,
                        &mut accumulated_cost,
                        step_start,
                        self.step_index,
                        &step_execs,
                    )
                    .await
                }
                _ => StepOutcome::Cancelled,
            };

            match outcome {
                StepOutcome::Completed => {
                    self.step_index += 1;
                }
                StepOutcome::Failed(msg) => {
                    let is_cancelled = *self.cancel_watch.borrow();
                    if is_cancelled {
                        // The step was failing (e.g. the agent process
                        // crashed mid-turn) when the user cancelled.
                        // Honour the cancellation: mark the step
                        // `interrupted` with the failure context, then
                        // cancel the feature. Without this branch the
                        // driver would exit silently and the UI would
                        // stay stuck on "running" because no event is
                        // emitted and the DB is never updated.
                        let wall = step_start.elapsed().as_secs();
                        let _ = self.features.step_update(
                            &step_exec.id,
                            &StepExecutionPatch {
                                iteration_count: None,
                                status: Some("interrupted".to_string()),
                                cost_usd: Some(accumulated_cost).map(|v| Some(v)),
                                wall_clock_secs: Some(wall).map(|v| Some(wall)),
                                artifact_path: None,
                                artifact_paths: None,
                                error_message: Some(Some(format!(
                                    "Cancelled while step was failing: {}",
                                    msg
                                ))),
                            },
                        );
                        let _ = self.notif.emit(&DomainEvent::StepProgress {
                            feature_id: self.f_id.clone(),
                            step_id: step_exec.step_id.0.clone(),
                            status: "interrupted".into(),
                            cost_usd: Some(accumulated_cost),
                            wall_clock_secs: Some(wall),
                        });
                        self.cancel_feature().await;
                    } else {
                        // Conditional edge: if the step declares an
                        // `on_failure -> goto <step>` and the retry
                        // budget allows, follow it. Otherwise fail the
                        // feature.
                        if let Some(redirect_idx) = self.evaluate_on_failure(
                            step_exec,
                            step_conf,
                            &msg,
                            accumulated_cost,
                            step_start,
                        ) {
                            self.step_index = redirect_idx;
                            // Continue the loop, do NOT return.
                            continue;
                        }
                        self.fail_step_and_feature(step_exec, &msg, accumulated_cost, step_start)
                            .await;
                    }
                    return;
                }
                StepOutcome::Cancelled => {
                    self.cancel_feature().await;
                    return;
                }
                StepOutcome::RedirectTo(idx) => {
                    self.step_index = idx;
                }
            }
        }

        // All steps completed
        let total_cost = self
            .features
            .steps_for_feature(&self.f_id)
            .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
            .unwrap_or(0.0);
        let total_dur = format!("{}s", self.start_time.elapsed().as_secs());
        let _ = self.features.update(
            &self.f_id,
            &FeaturePatch {
                status: Some("completed".to_string()),
                total_cost: Some(total_cost).map(|v| Some(v)),
                duration: Some(&total_dur).map(|v| Some(v.to_string())),
                ..Default::default()
            },
        );
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: self.f_id.clone(),
            status: "completed".into(),
        });
    }

    async fn fail_step_and_feature(
        &self,
        step_exec: &StepExecution,
        msg: &str,
        accumulated_cost: f64,
        step_start: Instant,
    ) {
        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                iteration_count: None,
                status: Some("failed".to_string()),
                cost_usd: Some(accumulated_cost).map(|v| Some(v)),
                wall_clock_secs: Some(wall).map(|v| Some(wall)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(Some(msg.to_string())),
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "failed".into(),
            cost_usd: Some(accumulated_cost),
            wall_clock_secs: Some(wall),
        });

        let total_cost = self
            .features
            .steps_for_feature(&self.f_id)
            .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
            .unwrap_or(0.0);
        let total_dur = format!("{}s", self.start_time.elapsed().as_secs());
        let _ = self.features.update(
            &self.f_id,
            &FeaturePatch {
                status: Some("failed".to_string()),
                total_cost: Some(total_cost).map(|v| Some(v)),
                duration: Some(&total_dur).map(|v| Some(v.to_string())),
                ..Default::default()
            },
        );
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: self.f_id.clone(),
            status: "failed".into(),
        });
    }

    async fn cancel_feature(&self) {
        let wall = self.start_time.elapsed().as_secs();
        let total_cost = self
            .features
            .steps_for_feature(&self.f_id)
            .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
            .unwrap_or(0.0);
        let total_dur = format!("{}s", wall);
        let _ = self.features.update(
            &self.f_id,
            &FeaturePatch {
                status: Some("cancelled".to_string()),
                total_cost: Some(total_cost).map(|v| Some(v)),
                duration: Some(&total_dur).map(|v| Some(v.to_string())),
                ..Default::default()
            },
        );
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: self.f_id.clone(),
            status: "cancelled".into(),
        });
    }

    /// Evaluate the conditional edge declared on a step (`on_failure ->
    /// goto <step>`) and the per-step retry budget (`max_iterations`).
    ///
    /// Returns `Some(target_index)` if the driver should follow the edge
    /// and continue the loop, or `None` if the step should be treated as
    /// terminal and the feature should fail.
    ///
    /// Rules (per `REDESIGN_PLAN.md` §1 decision 8 + `AGENT_INTEGRATION.md`
    /// §3.5):
    ///
    /// - No `on_failure` set: terminal → return `None`.
    /// - Target step id doesn't resolve to a known step: log a warning,
    ///   return `None` (don't infinite-loop on a typo).
    /// - `iteration_count + 1` exceeds `max_iterations`: terminal → return
    ///   `None` and persist a clear "budget exhausted" error.
    /// - Otherwise: bump the persisted `iteration_count` on the *failing*
    ///   step (so the audit trail shows how many retries were used), mark
    ///   the step `failed` with a retry-pending hint, and return
    ///   `Some(target_index)`.
    fn evaluate_on_failure(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        msg: &str,
        accumulated_cost: f64,
        step_start: Instant,
    ) -> Option<usize> {
        let target_id = match step_conf.on_failure.as_ref() {
            Some(id) if !id.0.is_empty() => id,
            _ => return None,
        };

        let max = step_conf.max_iterations.unwrap_or(0);
        let already = step_exec.iteration_count;
        if already + 1 > max {
            // Budget exhausted. Persist a clear error on the step so the
            // user understands why the loop ended.
            let wall = step_start.elapsed().as_secs();
            let _ = self.features.step_update(
                &step_exec.id,
                &StepExecutionPatch {
                    iteration_count: None,
                    status: Some("failed".to_string()),
                    cost_usd: Some(accumulated_cost).map(|v| Some(v)),
                    wall_clock_secs: Some(wall).map(|v| Some(wall)),
                    artifact_path: None,
                    artifact_paths: None,
                    error_message: Some(Some(format!(
                        "{} (retry budget exhausted: {} of {} attempts on '{}')",
                        msg, already, max, target_id.0
                    ))),
                },
            );
            let _ = self.notif.emit(&DomainEvent::StepProgress {
                feature_id: self.f_id.clone(),
                step_id: step_exec.step_id.0.clone(),
                status: "failed".into(),
                cost_usd: Some(accumulated_cost),
                wall_clock_secs: Some(wall),
            });
            return None;
        }

        let target_idx = self.steps.iter().position(|s| s.id == *target_id)?;
        // Bump the failing step's iteration counter so the next time we
        // hit this branch we know we've used one of the budgeted retries.
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                status: Some("failed".to_string()),
                cost_usd: Some(accumulated_cost).map(|v| Some(v)),
                artifact_path: None,
                artifact_paths: None,
                error_message: Some(Some(format!(
                    "{} (retrying: will jump to '{}' on attempt {} of {})",
                    msg,
                    target_id.0,
                    already + 1,
                    max
                ))),
                iteration_count: Some(already + 1),
                ..Default::default()
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "failed".into(),
            cost_usd: Some(accumulated_cost),
            wall_clock_secs: Some(step_start.elapsed().as_secs()),
        });
        Some(target_idx)
    }
}

// Step handler methods live in:
//   - steps/agent.rs    → handle_agent_step
//   - steps/gate.rs     → handle_gate_step
//   - steps/parallel.rs → handle_parallel_step
