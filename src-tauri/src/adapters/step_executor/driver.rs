use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use tokio::sync::watch;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepConfig;
use crate::domain::prompt_context::PromptContext;
use crate::ports::agent_execution::AgentExecutionPort;
use crate::ports::artifact_store::ArtifactStore;
use crate::ports::db::FeatureRepository;
use crate::ports::execution::ExecutionPort;
use crate::ports::merge::MergeExecutor;
use crate::ports::notification::NotificationPort;

pub(crate) mod failure;
pub(crate) mod verifier;
pub(crate) use super::driver_registry::DriverRegistry;

/// The default `on_failure` retry-loop budget when neither the run override
/// (`Feature::loop_iterations`), the project setting
/// (`ProjectSettings::default_loop_iterations`), nor the step's own
/// `max_iterations` is set.
pub(crate) const DEFAULT_LOOP_ITERATIONS: u32 = 3;

/// Feedback captured when a step fails and the loop redirects back to an
/// earlier step. Injected into the retried step's prompt as
/// `{{retry_feedback}}` / `{{iteration}}` / `{{max_iterations}}` so the
/// retry isn't blind. Held in-memory for the lifetime of a single run.
#[derive(Clone)]
pub(crate) struct RetryContext {
    /// Raw failure / verifier reason from the step that triggered the loop.
    pub feedback: String,
    /// 1-based attempt number we're now starting.
    pub iteration: u32,
    /// Effective max iterations for this loop.
    pub max: u32,
}

/// Holds all shared state for a single feature execution run.
pub(crate) struct ExecutionDriver {
    // Repository / service Arcs
    pub features: Arc<dyn FeatureRepository>,
    pub gates: Arc<dyn crate::ports::db::GateRepository>,
    pub projects: Arc<dyn crate::ports::db::ProjectRepository>,
    pub signals: Arc<dyn crate::ports::memory_signals::MemorySignalsPort>,
    pub notif: Arc<dyn NotificationPort>,
    /// Notification persistence port. The driver uses this to
    /// write a row to the `notifications` table when a user-visible
    /// event is emitted from inside a step (e.g. retry budget
    /// exhausted). Mirrors the same `SqliteAdapter` instance as
    /// `features` / `gates`; no separate I/O.
    pub notifications: Arc<dyn crate::ports::db::NotificationRepository>,
    pub registry: Arc<AgentRegistry>,
    pub agent_exec: Arc<dyn AgentExecutionPort>,
    pub exec: Arc<dyn ExecutionPort>,
    pub artifacts: Arc<dyn ArtifactStore>,
    pub app_local_data_dir: PathBuf,
    pub git_ops: GitOpsHelper,
    pub merge_executor: Arc<dyn MergeExecutor>,
    pub gate_waiters: Arc<Mutex<HashMap<String, Arc<GateWaiter>>>>,
    pub driver_registry: Arc<DriverRegistry>,

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

    /// Repo-relative folder where agents write their reports.
    /// Snapshotted at feature-start time from the project settings
    /// (and the Feature row's per-feature override). The driver
    /// passes this to every `commit_worktree_changes` call so the
    /// orchestrator can include or exclude the folder from the
    /// commit depending on `commit_artifacts`. See migration V12
    /// and `commit_worktree_changes` in
    /// `artifacts/declared.rs`.
    pub artifact_subdir: String,

    /// Whether to include `artifact_subdir` in
    /// `commit_worktree_changes`. `true` → reports land in the PR.
    /// `false` → reports stay in demeteo's `FsArtifactStore` only.
    /// Resolved at feature-start time as
    /// `features.commit_artifacts ?? settings.commit_artifacts`.
    pub commit_artifacts: bool,

    // --- Agent/model resolution inputs (snapshotted at feature start) ---
    /// Feature-wide run override of the agent kind (the run modal's
    /// "apply to all"). Beats the workflow step but loses to a per-step
    /// override. `None` = not set.
    pub feature_agent_kind: Option<String>,
    /// Feature-wide run override of the model. Same precedence as
    /// `feature_agent_kind`.
    pub feature_model: Option<String>,
    /// Per-step agent/model overrides chosen at launch (highest precedence).
    pub step_overrides: Vec<crate::domain::models::StepOverride>,
    /// Project default agent kind (`ProjectSettings::default_agent_kind`).
    pub default_agent_kind: Option<String>,
    /// Project default model (`ProjectSettings::default_model`).
    pub default_model: Option<String>,

    // --- Loop budget inputs ---
    /// Per-run override of the loop budget (`Feature::loop_iterations`).
    pub loop_iterations_override: Option<u32>,
    /// Project default loop budget (`ProjectSettings::default_loop_iterations`).
    pub project_default_loop_iterations: Option<u32>,

    /// Set when a step fails and the loop redirects to an earlier step;
    /// consumed by the next step's prompt build, then cleared.
    pub retry_ctx: Option<RetryContext>,
}

impl ExecutionDriver {
    /// Capture a raw run observation for the memory agent's queue. Best-effort:
    /// an empty body, a missing feature row, or an enqueue failure is silently
    /// swallowed so signal capture never perturbs the run itself.
    pub(crate) fn capture_signal(
        &self,
        step_execution_id: Option<String>,
        kind: crate::domain::memory::SignalKind,
        content: impl Into<String>,
    ) {
        let content = content.into();
        if content.trim().is_empty() {
            return;
        }
        let project_id = match self.features.get(&self.f_id) {
            Ok(Some(f)) => f.project_id,
            _ => return,
        };
        let now = crate::paths::now_ms();
        let signal = crate::domain::memory::MemorySignal {
            id: format!("ms-{}", crate::paths::new_id()),
            project_id,
            feature_id: self.f_id_str.clone(),
            step_execution_id,
            kind,
            content,
            created_at: now,
            processed_at: None,
            attempts: 0,
        };
        let _ = self.signals.enqueue(signal);
    }

    /// Resolve the effective `(agent_kind, model)` for a given step.
    ///
    /// Precedence (first non-empty wins):
    ///   per-step run override → feature-wide run override → workflow step
    ///   → project default → built-in (`"opencode"` for the agent; no model).
    pub(crate) fn resolve_step_agent(&self, step_conf: &StepConfig) -> (String, Option<String>) {
        let ov = self
            .step_overrides
            .iter()
            .find(|o| o.step_id == step_conf.id.0);
        resolve_agent_model(
            ov,
            self.feature_agent_kind.as_deref(),
            self.feature_model.as_deref(),
            step_conf,
            self.default_agent_kind.as_deref(),
            self.default_model.as_deref(),
        )
    }

    /// Effective loop-iteration budget for a step with `on_failure` set.
    /// Precedence: run override → project default → step `max_iterations`
    /// → engine default (3).
    pub(crate) fn effective_loop_iterations(&self, step_conf: &StepConfig) -> u32 {
        resolve_loop_iterations(
            self.loop_iterations_override,
            self.project_default_loop_iterations,
            step_conf.max_iterations,
        )
    }
}

/// Pure agent/model resolution. Precedence (first non-empty wins):
/// per-step run override → feature-wide run override → workflow step →
/// project default → built-in (`"opencode"` agent; no model).
pub(crate) fn resolve_agent_model(
    step_override: Option<&crate::domain::models::StepOverride>,
    feature_agent: Option<&str>,
    feature_model: Option<&str>,
    step_conf: &StepConfig,
    default_agent: Option<&str>,
    default_model: Option<&str>,
) -> (String, Option<String>) {
    let agent = step_override
        .and_then(|o| o.agent_kind.clone())
        .or_else(|| feature_agent.map(str::to_string))
        .or_else(|| step_conf.agent_kind.clone())
        .or_else(|| default_agent.map(str::to_string))
        .unwrap_or_else(|| "opencode".to_string());

    let model = step_override
        .and_then(|o| o.model.clone())
        .or_else(|| feature_model.map(str::to_string))
        .or_else(|| step_conf.model.clone())
        .or_else(|| default_model.map(str::to_string));

    (agent, model)
}

/// Pure loop-budget resolution: run override → project default → step
/// `max_iterations` → engine default (3).
pub(crate) fn resolve_loop_iterations(
    run_override: Option<u32>,
    project_default: Option<u32>,
    step_max: Option<u32>,
) -> u32 {
    run_override
        .or(project_default)
        .or(step_max)
        .unwrap_or(DEFAULT_LOOP_ITERATIONS)
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
            // Clone `step_conf` so it doesn't borrow `self.steps` —
            // `handle_gate_step` now takes `&mut self` (it sets
            // `retry_ctx` on a redirect with feedback), and the borrow
            // checker won't let us hold an immutable borrow across
            // that call.
            let step_conf = match self.steps.iter().find(|s| s.id == step_exec.step_id) {
                Some(sc) => sc.clone(),
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
                        &step_conf,
                        &mut accumulated_cost,
                        &mut accumulated_tokens,
                        step_start,
                        self.step_index,
                        &step_execs,
                    )
                    .await
                }
                "gate" => {
                    // Clone `step_conf` to release the immutable borrow
                    // on `self.steps` — `handle_gate_step` now takes
                    // `&mut self` so it can populate `retry_ctx` when a
                    // redirect carries feedback.
                    let step_conf = step_conf.clone();
                    self.handle_gate_step(
                        step_exec,
                        &step_conf,
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
                        &step_conf,
                        &mut accumulated_cost,
                        &mut accumulated_tokens,
                        step_start,
                        self.step_index,
                        &step_execs,
                    )
                    .await
                }
                "sync" => {
                    self.handle_sync_step(step_exec, &step_conf, &mut accumulated_cost, step_start)
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
                    // Retry feedback is scoped to the single redirected step;
                    // once it completes, clear it so later steps don't inherit
                    // stale feedback.
                    self.retry_ctx = None;
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
                            &step_conf,
                            &msg,
                            accumulated_cost,
                            accumulated_tokens,
                            step_start,
                        ) {
                            // Capture the failure so the retried step's prompt
                            // isn't blind. `iteration_count` was just bumped to
                            // `already + 1` in evaluate_on_failure, so the
                            // attempt now starting is that value.
                            let max = self.effective_loop_iterations(&step_conf);
                            let iteration = step_exec.iteration_count + 1;
                            let feedback = msg.clone();
                            self.capture_signal(
                                Some(step_exec.id.0.clone()),
                                crate::domain::memory::SignalKind::Retry,
                                format!(
                                    "Step '{}' failed (attempt {} of {}), retrying: {}",
                                    step_exec.step_id.0, iteration, max, msg
                                ),
                            );
                            self.retry_ctx = Some(RetryContext {
                                feedback,
                                iteration,
                                max,
                            });
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

        // Drop any stale gate waiter left behind — the loop above
        // consumes them on success, but cancellation / failure paths
        // can leak. Idempotent; an already-absent entry is fine.
        self.gate_waiters.lock().unwrap().clear();

        // Deregister so a follow-up `ensure_driver_running` for this
        // feature knows to start a fresh driver instead of trusting a
        // (now-completed) registry entry.
        self.driver_registry.deregister(&self.f_id);
    }
}

#[cfg(test)]
mod resolution_tests {
    use super::{resolve_agent_model, resolve_loop_iterations};
    use crate::domain::ids::StepId;
    use crate::domain::models::{StepConfig, StepOverride};

    fn step(agent: Option<&str>, model: Option<&str>) -> StepConfig {
        StepConfig {
            id: StepId::from("s-impl".to_string()),
            kind: "agent".to_string(),
            title: "Implement".to_string(),
            agent_kind: agent.map(str::to_string),
            model: model.map(str::to_string),
            prompt_template: None,
            artifact_mode: "full".to_string(),
            on_failure: None,
            max_iterations: None,
            artifacts: None,
            verifier: None,
            capability: None,
            allow_network: false,
            allow_shell: false,
        }
    }

    #[test]
    fn per_step_override_wins() {
        let ov = StepOverride {
            step_id: "s-impl".to_string(),
            agent_kind: Some("claude-code".to_string()),
            model: Some("claude-opus-4-8".to_string()),
        };
        let (a, m) = resolve_agent_model(
            Some(&ov),
            Some("hermes"),
            Some("feat-model"),
            &step(Some("opencode"), Some("step-model")),
            Some("opencode"),
            Some("proj-model"),
        );
        assert_eq!(a, "claude-code");
        assert_eq!(m.as_deref(), Some("claude-opus-4-8"));
    }

    #[test]
    fn falls_through_to_workflow_then_project_then_default() {
        // No per-step, no feature-wide → workflow step value wins.
        let (a, m) = resolve_agent_model(
            None,
            None,
            None,
            &step(Some("claude-code"), None),
            Some("opencode"),
            Some("proj-model"),
        );
        assert_eq!(a, "claude-code");
        // model: step has none → project default fills it.
        assert_eq!(m.as_deref(), Some("proj-model"));

        // Nothing set anywhere → built-in opencode, no model.
        let (a2, m2) = resolve_agent_model(None, None, None, &step(None, None), None, None);
        assert_eq!(a2, "opencode");
        assert_eq!(m2, None);
    }

    #[test]
    fn feature_wide_beats_workflow_but_loses_to_per_step() {
        let (a, _) = resolve_agent_model(
            None,
            Some("hermes"),
            None,
            &step(Some("opencode"), None),
            None,
            None,
        );
        assert_eq!(a, "hermes");
    }

    #[test]
    fn loop_budget_precedence() {
        assert_eq!(resolve_loop_iterations(Some(7), Some(5), Some(2)), 7);
        assert_eq!(resolve_loop_iterations(None, Some(5), Some(2)), 5);
        assert_eq!(resolve_loop_iterations(None, None, Some(2)), 2);
        assert_eq!(resolve_loop_iterations(None, None, None), 3);
    }
}
