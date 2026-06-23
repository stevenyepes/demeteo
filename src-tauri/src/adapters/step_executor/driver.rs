use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{oneshot, watch};

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::{GateDecision, StepConfig};
use crate::domain::prompt_context::PromptContext;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::FeatureRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::NotificationPort;

pub(crate) mod failure;
pub(crate) mod verifier;

/// Holds all shared state for a single feature execution run.
pub(crate) struct ExecutionDriver {
    // Repository / service Arcs
    pub features: Arc<dyn FeatureRepository>,
    pub gates: Arc<dyn crate::ports::db::GateRepository>,
    pub projects: Arc<dyn crate::ports::db::ProjectRepository>,
    pub memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
    pub notif: Arc<dyn NotificationPort>,
    pub registry: Arc<AgentRegistry>,
    pub agent_exec: Arc<dyn AgentExecutionPort>,
    pub exec: Arc<dyn ExecutionPort>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub app_local_data_dir: PathBuf,
    pub git_ops: GitOpsHelper,
    pub merge_executor: Arc<dyn MergeExecutor>,
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

            super::updates::update_step_status(
                &*self.features,
                &*self.notif,
                step_exec,
                &self.f_id,
                "running",
                step_exec.cost_usd.unwrap_or(0.0),
                step_exec.tokens,
                step_exec.wall_clock_secs.unwrap_or(0),
                None,
                None,
            );

            let step_start = Instant::now();
            let mut accumulated_cost = step_exec.cost_usd.unwrap_or(0.0);
            let mut accumulated_tokens = step_exec.tokens.unwrap_or(0);

            let outcome = match step_conf.kind.as_str() {
                "agent" => {
                    self.handle_agent_step(
                        step_exec,
                        step_conf,
                        &mut accumulated_cost,
                        &mut accumulated_tokens,
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
                        &mut accumulated_tokens,
                        step_start,
                        self.step_index,
                        &step_execs,
                    )
                    .await
                }
                "sync" => {
                    self.handle_sync_step(step_exec, step_conf, &mut accumulated_cost, step_start)
                        .await
                }
                other => {
                    let msg = format!("Unknown step kind: {}", other);
                    self.fail_step_and_feature(
                        step_exec,
                        &msg,
                        accumulated_cost,
                        accumulated_tokens,
                        step_start,
                    )
                    .await;
                    return;
                }
            };

            match outcome {
                crate::adapters::step_executor::steps::StepOutcome::Completed => {
                    let wall = step_start.elapsed().as_secs();
                    let latest_step = self.features.step_get(&step_exec.id).ok().flatten();
                    let art_path = latest_step.as_ref().and_then(|s| s.artifact_path.clone());
                    super::updates::update_step_status(
                        &*self.features,
                        &*self.notif,
                        step_exec,
                        &self.f_id,
                        "completed",
                        accumulated_cost,
                        Some(accumulated_tokens),
                        wall,
                        art_path,
                        None,
                    );
                    self.step_index += 1;
                }
                crate::adapters::step_executor::steps::StepOutcome::Failed(msg) => {
                    let is_cancelled = *self.cancel_watch.borrow();
                    if is_cancelled {
                        let wall = step_start.elapsed().as_secs();
                        super::updates::update_step_status(
                            &*self.features,
                            &*self.notif,
                            step_exec,
                            &self.f_id,
                            "interrupted",
                            accumulated_cost,
                            Some(accumulated_tokens),
                            wall,
                            None,
                            Some(format!("Cancelled while step was failing: {}", msg)),
                        );
                        self.cancel_feature().await;
                    } else {
                        if let Some(redirect_idx) = self.evaluate_on_failure(
                            step_exec,
                            step_conf,
                            &msg,
                            accumulated_cost,
                            accumulated_tokens,
                            step_start,
                        ) {
                            self.step_index = redirect_idx;
                            continue;
                        }
                        self.fail_step_and_feature(
                            step_exec,
                            &msg,
                            accumulated_cost,
                            accumulated_tokens,
                            step_start,
                        )
                        .await;
                    }
                    return;
                }
                crate::adapters::step_executor::steps::StepOutcome::Cancelled => {
                    self.cancel_feature().await;
                    return;
                }
                crate::adapters::step_executor::steps::StepOutcome::RedirectTo(idx) => {
                    self.step_index = idx;
                }
            }
        }

        let target_status = match self.features.get(&self.f_id) {
            Ok(Some(f)) if f.mr_url.as_ref().is_some_and(|u| !u.is_empty()) => "completed",
            _ => "awaiting_mr",
        };

        super::updates::finish_feature(
            &*self.features,
            &*self.notif,
            &self.f_id,
            target_status,
            self.start_time,
        );
    }
}
