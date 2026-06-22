use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::{oneshot, watch};

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::agent_event::AgentEvent;
use crate::domain::ids::FeatureId;
use crate::domain::models::{GateDecision, StepConfig, StepExecution};
use crate::domain::prompt_context::PromptContext;
use crate::paths;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::agent_runtime::AgentContext;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::{
    FeaturePatch, FeatureRepository, GateRepository, ProjectRepository, StepExecutionPatch,
};
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::{DomainEvent, NotificationPort};
use tokio_stream::StreamExt;

/// Holds all shared state for a single feature execution run.
/// Fields are `pub(crate)` so step-handler `impl` blocks in child modules can access them.
pub(crate) struct ExecutionDriver {
    // Repository / service Arcs
    pub features: Arc<dyn FeatureRepository>,
    pub gates: Arc<dyn GateRepository>,
    pub projects: Arc<dyn ProjectRepository>,
    pub memory: Arc<dyn crate::ports::memory::ProjectMemoryPort>,
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
                    cost_usd: step_exec.cost_usd.map(Some),
                    wall_clock_secs: step_exec.wall_clock_secs.map(Some),
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
                                cost_usd: Some(Some(accumulated_cost)),
                                wall_clock_secs: Some(Some(wall)),
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
                total_cost: Some(Some(total_cost)),
                duration: Some(Some(total_dur.clone())),
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
                cost_usd: Some(Some(accumulated_cost)),
                wall_clock_secs: Some(Some(wall)),
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
                total_cost: Some(Some(total_cost)),
                duration: Some(Some(total_dur.clone())),
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
                total_cost: Some(Some(total_cost)),
                duration: Some(Some(total_dur.clone())),
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
    /// Rules (per `docs/DECISIONS.md` decision 8 + `AGENT_INTEGRATION.md`
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
                    cost_usd: Some(Some(accumulated_cost)),
                    wall_clock_secs: Some(Some(wall)),
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
                cost_usd: Some(Some(accumulated_cost)),
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

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn run_verifier_logic(
        &self,
        step_exec: &StepExecution,
        verifier_cfg: &crate::domain::verifier::VerifierConfig,
        wt_path: &str,
        produced_artifacts: &[crate::domain::artifact::Artifact],
        accumulated_cost: &mut f64,
        step_start: Instant,
        default_agent_kind: &str,
        override_model: &Option<String>,
        machine_str: &str,
    ) -> Result<(), String> {
        // Emit verification status
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "verifying".into(),
            cost_usd: Some(*accumulated_cost),
            wall_clock_secs: Some(step_start.elapsed().as_secs()),
        });

        // 1. Resolve test harness command
        let feature = self.features.get(&self.f_id).ok().flatten();
        let mut harnesses = None;
        if let Some(ref f) = feature {
            if let Ok(Some(settings)) = self.projects.get_settings(&f.project_id) {
                harnesses = settings.worktree_strategy.harnesses;
            }
        }

        let (harness_name, harness_cmd) = {
            let name = verifier_cfg
                .harness_name
                .clone()
                .unwrap_or_else(|| "default".to_string());
            let command = verifier_cfg
                .harness_name
                .as_ref()
                .and_then(|name| harnesses.as_ref().and_then(|h| h.get(name)))
                .cloned()
                .or_else(|| {
                    feature.as_ref().and_then(|f| {
                        self.projects
                            .get_settings(&f.project_id)
                            .ok()
                            .flatten()
                            .and_then(|s| s.worktree_strategy.test_command.clone())
                    })
                })
                .unwrap_or_else(|| "npm test".to_string());
            (name, command)
        };

        // 2. Run harness command
        let harness_run_cmd = format!(
            "cd {} && {}",
            paths::shell_escape_posix(wt_path),
            harness_cmd
        );
        let (_harness_success, harness_output) =
            match self.exec.run_command(machine_str, &harness_run_cmd) {
                Ok(out) => (true, out),
                Err(out) => (false, out),
            };

        // 3. Prepare verifier prompt
        let mut produced_artifacts_summary = String::new();
        for art in produced_artifacts {
            produced_artifacts_summary.push_str(&format!("- File/Artifact: {}\n", art.name));
        }

        let verifier_prompt = format!(
            "You are a verifier agent performing a verification task.\n\n\
             Instructions:\n\
             {}\n\n\
             We ran the test harness '{}' with the command '{}'.\n\
             The output of the test command was:\n\
             ```\n\
             {}\n\
             ```\n\n\
             We also produced/modified the following files/artifacts:\n\
             {}\n\n\
             Please analyze the test output and artifacts, then provide a JSON object containing the verification verdict.\n\
             The JSON object must have a key '{}' with the value either \"pass\" or \"fail\".\n\
             For example: {{ \"{}\": \"pass\" }} or {{ \"{}\": \"fail\", \"reason\": \"...\" }}.\n\
             Do not output any other text or code blocks outside the JSON.",
            verifier_cfg.instructions,
            harness_name,
            harness_cmd,
            harness_output,
            produced_artifacts_summary,
            verifier_cfg.verdict_key,
            verifier_cfg.verdict_key,
            verifier_cfg.verdict_key,
        );

        // 4. Construct verifier agent context
        let verifier_agent_kind = verifier_cfg
            .agent_kind
            .clone()
            .unwrap_or_else(|| default_agent_kind.to_string());

        let mut agent_env = crate::ports::agent_runtime::agent_base_env();
        if let Some(ref m) = override_model {
            if verifier_agent_kind != "opencode" && verifier_agent_kind != "hermes" {
                let config = format!(
                    r#"{{"$schema":"https://opencode.ai/config.json","model":"{}"}}"#,
                    m
                );
                agent_env.insert("OPENCODE_CONFIG_CONTENT".to_string(), config);
            }
        }

        let verifier_thread_id = format!("{}-verifier", self.f_id_str);
        let verifier_ctx = AgentContext {
            thread_id: verifier_thread_id.clone(),
            machine_id: machine_str.to_string(),
            binary: verifier_agent_kind.clone(),
            args: vec![],
            env: agent_env,
            cwd: wt_path.to_string(),
            model: override_model.clone(),
            title: Some(format!("Verify: {}", harness_name)),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
        };

        // 5. Spawn and prompt the verifier agent
        let spawn_fut =
            self.registry
                .get_or_spawn(&verifier_thread_id, &verifier_agent_kind, verifier_ctx);
        let mut cancel_watch_spawn = self.cancel_watch.clone();
        let spawn_res = tokio::select! {
            res = spawn_fut => Some(res),
            _ = cancel_watch_spawn.changed() => None,
        };

        let session = match spawn_res {
            Some(Ok(session)) => session,
            Some(Err(e)) => return Err(format!("Verifier spawn failed: {}", e)),
            None => return Err("Verifier spawn cancelled".to_string()),
        };

        let mut text_buffer = String::new();
        let hb = session.stderr_heartbeat();
        let mut stream = session.prompt(&verifier_prompt);
        let mut cancel_watch = self.cancel_watch.clone();
        let mut first_event_seen = false;

        const VERIFIER_TIMEOUT_S: u64 = 180;
        let fast_sleep = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_S));
        let normal_sleep = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_S));
        let wall_sleep = tokio::time::sleep(std::time::Duration::from_secs(VERIFIER_TIMEOUT_S * 2));
        tokio::pin!(fast_sleep);
        tokio::pin!(normal_sleep);
        tokio::pin!(wall_sleep);

        let mut run_failed = None;
        let mut run_cancelled = false;

        loop {
            tokio::select! {
                event_opt = stream.next() => {
                    let event = match event_opt {
                        Some(ev) => ev,
                        None => break,
                    };
                    first_event_seen = true;

                    let now = tokio::time::Instant::now();
                    let next_fast = now + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S);
                    let next_normal = now + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S);
                    fast_sleep.as_mut().reset(next_fast);
                    normal_sleep.as_mut().reset(next_normal);

                    match event {
                        AgentEvent::Text { delta } => {
                            let _ = self.notif.emit(&DomainEvent::AgentStream {
                                feature_id: self.f_id.clone(),
                                step_execution_id: step_exec.id.clone(),
                                content: delta.clone(),
                            });
                            text_buffer.push_str(&delta);
                        }
                        AgentEvent::Usage { cost_usd: Some(c), .. } => {
                            *accumulated_cost += c;
                        }
                        AgentEvent::TurnComplete { .. } => break,
                        AgentEvent::Error { message, .. } => {
                            run_failed = Some(format!("Verifier agent error: {}", message));
                            break;
                        }
                        _ => {}
                    }
                }
                _ = &mut fast_sleep => {
                    if !first_event_seen {
                        fast_sleep.as_mut().reset(
                            tokio::time::Instant::now() + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S),
                        );
                        continue;
                    }
                    if hb.as_ref().is_some_and(|h| h.last_activity_ago_ms() > VERIFIER_TIMEOUT_S * 1000) {
                        run_failed = Some("Verifier blocked: no output (stdout and stderr silent)".to_string());
                        break;
                    }
                    fast_sleep.as_mut().reset(
                        tokio::time::Instant::now() + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S),
                    );
                }
                _ = &mut normal_sleep => {
                    if let Some(ref h) = hb {
                        if h.last_activity_ago_ms() < VERIFIER_TIMEOUT_S * 1000 {
                            normal_sleep.as_mut().reset(
                                tokio::time::Instant::now() + std::time::Duration::from_secs(VERIFIER_TIMEOUT_S),
                            );
                            continue;
                        }
                    }
                    run_failed = Some("Verifier response timed out".to_string());
                    break;
                }
                _ = &mut wall_sleep => {
                    run_failed = Some("Verifier exceeded wall clock cap".to_string());
                    break;
                }
                _ = cancel_watch.changed() => {
                    if *cancel_watch.borrow() {
                        let _ = session.cancel();
                        run_cancelled = true;
                        break;
                    }
                }
            }
        }

        let _ = self.registry.kill(&verifier_thread_id).await;

        if run_cancelled || *self.cancel_watch.borrow() {
            return Err("Verifier cancelled by user".to_string());
        }

        if let Some(err) = run_failed {
            return Err(err);
        }

        // 6. Parse verdict JSON
        let start = text_buffer.find('{');
        let end = text_buffer.rfind('}');
        if let (Some(s), Some(e)) = (start, end) {
            if s < e {
                let json_str = &text_buffer[s..=e];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(v_obj) = val.as_object() {
                        if let Some(verdict_val) = v_obj.get(&verifier_cfg.verdict_key) {
                            if let Some(verdict_str) = verdict_val.as_str() {
                                let verdict_lower = verdict_str.to_lowercase();
                                if verdict_lower == "pass" {
                                    return Ok(());
                                } else if verdict_lower == "fail" {
                                    let reason = v_obj
                                        .get("reason")
                                        .and_then(|r| r.as_str())
                                        .unwrap_or("Verifier agent verdict is 'fail'")
                                        .to_string();
                                    return Err(reason);
                                }
                            }
                        }
                    }
                }
            }
        }

        Err(format!("Could not parse valid verification JSON from verifier agent response. Response was: {}", text_buffer))
    }
}

// Step handler methods live in:
//   - steps/agent.rs    → handle_agent_step
//   - steps/gate.rs     → handle_gate_step
//   - steps/parallel.rs → handle_parallel_step
