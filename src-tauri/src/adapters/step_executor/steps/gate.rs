use std::time::Instant;

use crate::adapters::step_executor::driver::{ExecutionDriver, RetryContext};
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::ids::{GateDecisionId, StepId};
use crate::domain::models::{GateDecision, StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

/// Inputs needed to apply a gate decision that the predecessor step
/// already produced. Bundled to keep [`ExecutionDriver::apply_gate_decision`]
/// below the clippy `too_many_arguments` threshold.
struct GateDecisionContext<'a> {
    step_exec: &'a StepExecution,
    step_conf: &'a StepConfig,
    /// All step executions for the current run, in order. The
    /// `redirect` branch needs this to reset the target step's
    /// status to `pending` so the driver doesn't skip it as
    /// already-completed on the next loop iteration.
    step_execs: &'a [StepExecution],
    prev_artifact_path: &'a Option<String>,
    prev_artifact_paths: &'a [String],
    accumulated_cost: &'a mut f64,
    step_start: Instant,
}

/// Resolve the redirect target for a `redirect` gate decision.
///
/// Priority:
///   1. Step ID in `feedback` (if it matches one of `steps`).
///   2. `on_failure` on the gate's step config.
///   3. The step immediately before the gate — i.e. the work the gate
///      was reviewing. This is the natural intent of "give the agent
///      my feedback and redo it" and stops the pipeline from silently
///      cancelling when the user types implementation feedback.
///   4. `None` only when the gate is the very first step.
fn resolve_redirect_target(
    steps: &[StepConfig],
    on_failure: Option<&StepId>,
    gate_step_index: u32,
    feedback: Option<&str>,
) -> Option<usize> {
    let explicit = feedback
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|cleaned| steps.iter().position(|s| s.id.0 == cleaned));

    explicit
        .or_else(|| on_failure.and_then(|id| steps.iter().position(|s| s.id == *id)))
        .or_else(|| {
            if gate_step_index > 0 {
                Some(gate_step_index as usize - 1)
            } else {
                None
            }
        })
}

/// Apply the durable state changes that a `redirect` gate decision
/// requires. Pulled out of [`ExecutionDriver::apply_gate_decision`]
/// so the loop-breaking fix is unit-testable without a full
/// `ExecutionDriver` (and so the in-line `apply_gate_decision`
/// branch stays a short redirect that delegates the work here).
///
/// Concretely:
///   * the target step is reset to status `pending` (with all
///     counters cleared and artifacts dropped) so the driver's
///     resume-skip logic does not treat it as already-completed and
///     skip past it; and
///   * the gate's own `gate_decisions` row is cleared so the next
///     visit to the gate re-prompts the user. Without this second
///     half, the gate's reconciliation would find the prior
///     `redirect` decision on file, return
///     `RedirectTo(target_idx)` once more, and the same step would
///     loop forever — the bug this fix exists to break.
///
/// Both writes are best-effort. Failures are intentionally
/// swallowed: the redirect already won the user's intent, and any
/// stale state is recoverable on the next reconciliation pass
/// (the startup watchdog will re-surface the gate if the driver
/// dies between the reset and the target step completing).
fn reset_for_redirect(
    features: &dyn crate::ports::db::FeatureRepository,
    gates: &dyn crate::ports::db::GateRepository,
    step_execs: &[StepExecution],
    target_idx: usize,
    gate_step_execution_id: &crate::domain::ids::StepExecutionId,
) {
    if let Some(target_exec) = step_execs.get(target_idx) {
        // Reset every counter / artifact the previous attempt
        // accumulated so the re-run starts from a clean slate.
        // `cost_usd` / `tokens` / `wall_clock_secs` are wrapped in
        // `Some(Some(0))` because the patch type uses
        // `Option<Option<T>>` to distinguish "leave alone" (`None`)
        // from "set to value" (`Some(Some(v))`).
        let _ = features.step_update(
            &target_exec.id,
            &StepExecutionPatch {
                iteration_count: None,
                status: Some("pending".to_string()),
                cost_usd: Some(Some(0.0)),
                tokens: Some(Some(0)),
                wall_clock_secs: Some(Some(0)),
                artifact_path: Some(None),
                artifact_paths: Some(Vec::new()),
                error_message: Some(None),
            },
        );
    }
    // Clear this gate's own decision row so the next visit to the
    // gate re-prompts the user. Idempotent against app restarts: if
    // the driver dies after the reset and before the target step
    // finishes, the startup watchdog will already mark the gate
    // `interrupted` and create a fresh `gate_decisions` row with
    // `decision = None` (see `startup_watchdog` in
    // `impl_traits/mod.rs`).
    let _ = gates.reset_for_step_execution(gate_step_execution_id);
}

impl ExecutionDriver {
    pub(crate) async fn handle_gate_step(
        &mut self,
        step_exec: &StepExecution,
        _step_conf: &StepConfig,
        accumulated_cost: &mut f64,
        step_start: Instant,
        step_index: usize,
        step_execs: &[StepExecution],
    ) -> StepOutcome {
        // Use the passed-in step_execs to get the previous step's artifact
        // list (avoids an extra DB call — the caller already fetched the
        // list). The gate inherits its predecessor's artifacts by default
        // so the UI can keep showing them on the gate card; if the user
        // redirects, the redirected step will re-derive the new lineage.
        let prev_artifact_path: Option<String> = if step_index > 0 {
            step_execs
                .get(step_index - 1)
                .and_then(|s| s.artifact_path.clone())
        } else {
            None
        };
        let prev_artifact_paths: Vec<String> = if step_index > 0 {
            step_execs
                .get(step_index - 1)
                .map(|s| s.artifact_paths.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Mark gate awaiting decision
        let wall = step_start.elapsed().as_secs();
        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                iteration_count: None,
                status: Some("awaiting_gate".to_string()),
                cost_usd: Some(Some(*accumulated_cost)),
                tokens: None,
                wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                artifact_path: prev_artifact_path.as_ref().map(|p| Some(p.clone())),
                artifact_paths: Some(prev_artifact_paths.clone()),
                error_message: Some(None),
            },
        );
        let _ = self.notif.emit(&DomainEvent::StepProgress {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            status: "awaiting_gate".into(),
            cost_usd: Some(*accumulated_cost),
            tokens: None,
            wall_clock_secs: Some(wall),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });

        // Ensure the gate_decisions row exists. `create` is idempotent
        // for the typical case (driver is mid-run); for the resume case
        // (`startup_watchdog` already inserted a row, or a previous run
        // did), we ignore the unique-constraint violation silently.
        let gate_dec_id = GateDecisionId::from(format!("gd-{}", step_exec.id.0));
        let gate_dec = GateDecision {
            id: gate_dec_id,
            step_execution_id: step_exec.id.clone(),
            decision: None,
            feedback: None,
            created_at: paths::now_ms(),
        };
        let _ = self.gates.create(gate_dec);

        // ── Reconciliation: did a decision already arrive while we
        // were dead? The DB row is the source of truth, so we always
        // check it before registering a fresh waiter. This is what
        // makes the system self-healing across app restarts and races.
        let recorded = self.gates.latest_for_step(&step_exec.id).ok().flatten();
        if let Some(rec) = recorded {
            if rec.decision.is_some() {
                let mut ctx = GateDecisionContext {
                    step_exec,
                    step_conf: _step_conf,
                    step_execs,
                    prev_artifact_path: &prev_artifact_path,
                    prev_artifact_paths: &prev_artifact_paths,
                    accumulated_cost,
                    step_start,
                };
                return self.apply_gate_decision(&rec, &mut ctx);
            }
        }

        let _ = self.notif.emit(&DomainEvent::GateRequired {
            feature_id: self.f_id.clone(),
            step_execution_id: step_exec.id.clone(),
        });

        // Set up waiter and wait for either a fresh decision or cancellation.
        let waiter = GateWaiter::new();
        self.gate_waiters
            .lock()
            .unwrap()
            .insert(step_exec.id.0.clone(), waiter.clone());

        let mut cancel_watch_gate = self.cancel_watch.clone();
        let decision = tokio::select! {
            d = waiter.wait() => d,
            _ = cancel_watch_gate.changed() => None,
        };

        // Remove our waiter regardless of how we woke up. A late
        // `gate_decide` that arrives after this point is handled by
        // upsert_decision + the next driver's reconciliation.
        self.gate_waiters.lock().unwrap().remove(&step_exec.id.0);

        let Some(decision) = decision else {
            return StepOutcome::Cancelled;
        };

        let mut ctx = GateDecisionContext {
            step_exec,
            step_conf: _step_conf,
            step_execs,
            prev_artifact_path: &prev_artifact_path,
            prev_artifact_paths: &prev_artifact_paths,
            accumulated_cost,
            step_start,
        };
        self.apply_gate_decision(&decision, &mut ctx)
    }

    /// Apply a recorded or freshly-delivered gate decision. Pure
    /// post-decision logic — no I/O discovery, no waiting. Reused by both
    /// the reconciliation path (decision was already in the DB when the
    /// driver woke up) and the in-memory wakeup path.
    fn apply_gate_decision(
        &mut self,
        decision_recvd: &GateDecision,
        ctx: &mut GateDecisionContext<'_>,
    ) -> StepOutcome {
        match decision_recvd.decision.as_deref() {
            Some("approve") => {
                if let Some(ref fb) = decision_recvd.feedback {
                    let cleaned = fb.trim();
                    if !cleaned.is_empty() {
                        self.capture_signal(
                            Some(ctx.step_exec.id.0.clone()),
                            crate::domain::memory::SignalKind::GateFeedback,
                            format!(
                                "Gate '{}' approved with feedback: {}",
                                ctx.step_exec.step_id.0, cleaned
                            ),
                        );
                    }
                }

                let wall = ctx.step_start.elapsed().as_secs();
                let _ = self.features.step_update(
                    &ctx.step_exec.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("completed".to_string()),
                        cost_usd: Some(Some(*ctx.accumulated_cost)),
                        tokens: None,
                        wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                        artifact_path: ctx.prev_artifact_path.as_ref().map(|p| Some(p.clone())),
                        artifact_paths: Some(ctx.prev_artifact_paths.to_vec()),
                        error_message: Some(None),
                    },
                );
                let _ = self.notif.emit(&DomainEvent::StepProgress {
                    feature_id: self.f_id.clone(),
                    step_id: ctx.step_exec.step_id.0.clone(),
                    status: "completed".into(),
                    cost_usd: Some(*ctx.accumulated_cost),
                    tokens: None,
                    wall_clock_secs: Some(wall),
                    cache_read_input_tokens: None,
                    cache_creation_input_tokens: None,
                });
                StepOutcome::Completed
            }
            Some("cancel") => StepOutcome::Failed("Gate Cancelled".to_string()),
            Some("redirect") => {
                let cleaned_feedback = decision_recvd
                    .feedback
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty());

                // If feedback isn't a step ID, capture it as a memory
                // signal so the next attempt has the user's guidance.
                // Capture runs *before* target resolution so a non-id
                // feedback doesn't get silently lost.
                if let Some(cleaned) = cleaned_feedback {
                    let matches_step = self.steps.iter().any(|s| s.id.0 == cleaned);
                    if !matches_step {
                        self.capture_signal(
                            Some(ctx.step_exec.id.0.clone()),
                            crate::domain::memory::SignalKind::GateFeedback,
                            format!(
                                "Gate '{}' redirected with instruction: {}",
                                ctx.step_exec.step_id.0, cleaned
                            ),
                        );
                    }
                }

                // Surface the user's feedback to the redirected step's
                // prompt via `retry_ctx`. Without this, `{{retry_feedback}}`
                // is empty for gate-driven redirects and the retried
                // agent sees no trace of the user's guidance until the
                // async memory agent distills the signal — far too
                // late. Setting it here makes the feedback appear in
                // the next step's prompt regardless of whether the
                // step's `prompt_template` references the variable
                // (the agent step also appends a "Previous Attempt
                // Feedback" section automatically when retry_ctx is
                // Some).
                if let Some(cleaned) = cleaned_feedback {
                    self.retry_ctx = Some(RetryContext {
                        feedback: cleaned.to_string(),
                        iteration: 1,
                        max: 1,
                    });
                }

                let target_idx = resolve_redirect_target(
                    &self.steps,
                    ctx.step_conf.on_failure.as_ref(),
                    ctx.step_exec.step_index,
                    decision_recvd.feedback.as_deref(),
                );

                match target_idx {
                    Some(idx) => {
                        // The gate redirected back to a previous step;
                        // reset that step's durable state and clear
                        // the gate's own decision row so the driver
                        // actually re-runs the target *and* re-prompts
                        // the user on the next gate visit. Skipping
                        // either half produces a loop: the spec
                        // would be re-run, but the gate would
                        // re-apply the prior `redirect` decision
                        // forever (the bug this helper fixes).
                        reset_for_redirect(
                            &*self.features,
                            &*self.gates,
                            ctx.step_execs,
                            idx,
                            &ctx.step_exec.id,
                        );
                        StepOutcome::RedirectTo(idx)
                    }
                    None => StepOutcome::Cancelled,
                }
            }
            _ => StepOutcome::Cancelled,
        }
    }
}

#[cfg(test)]
mod redirect_target_tests {
    use super::*;
    use crate::domain::ids::StepId;

    fn step(id: &str) -> StepConfig {
        StepConfig {
            id: StepId::from(id.to_string()),
            kind: "agent".to_string(),
            title: id.to_string(),
            agent_kind: None,
            model: None,
            prompt_template: None,
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
    fn explicit_step_id_in_feedback_wins() {
        let steps = vec![
            step("research"),
            step("spec"),
            step("gate"),
            step("implement"),
        ];
        let target = resolve_redirect_target(&steps, None, 2, Some("implement"));
        assert_eq!(target, Some(3));
    }

    #[test]
    fn free_text_feedback_falls_back_to_previous_step() {
        // The user's bug: typing implementation feedback used to
        // silently cancel the pipeline. The fallback should land on
        // the step immediately before the gate.
        let steps = vec![
            step("research"),
            step("spec"),
            step("gate"),
            step("implement"),
        ];
        let target = resolve_redirect_target(
            &steps,
            None,
            2, // gate is at index 2
            Some("make sure to use cargo before mise"),
        );
        assert_eq!(target, Some(1), "should fall back to the spec step");
    }

    #[test]
    fn on_failure_takes_priority_over_previous_step() {
        let steps = vec![
            step("research"),
            step("spec"),
            step("gate"),
            step("implement"),
        ];
        let target = resolve_redirect_target(
            &steps,
            Some(&StepId::from("research".to_string())),
            2,
            Some("random feedback"),
        );
        assert_eq!(target, Some(0));
    }

    #[test]
    fn gate_at_step_zero_cancels() {
        let steps = vec![step("gate")];
        let target = resolve_redirect_target(&steps, None, 0, Some("feedback"));
        assert_eq!(target, None);
    }

    #[test]
    fn empty_feedback_with_no_on_failure_falls_back() {
        let steps = vec![step("research"), step("gate")];
        let target = resolve_redirect_target(&steps, None, 1, Some("   "));
        assert_eq!(target, Some(0));
    }
}

/// The bug this regression suite exists to break: when a gate
/// redirects back to a previous step with feedback, the orchestrator
/// used to re-run the target step, then re-enter the gate, find the
/// same `redirect` decision on file, redirect back again — and loop
/// forever. `reset_for_redirect` is the fix: it resets the target
/// step's status to `pending` and clears the gate's own decision
/// row. These tests pin both halves of the fix in place.
#[cfg(test)]
mod redirect_reset_tests {
    use super::*;
    use crate::adapters::database::SqliteAdapter;
    use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, WorkflowId};
    use crate::domain::models::Feature;
    use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
    use rusqlite::Connection;

    /// Construct an in-memory `SqliteAdapter` that implements every
    /// port the helper touches (`FeatureRepository`,
    /// `GateRepository`, and the rest of the trait surface that
    /// `SqliteAdapter::new` requires). We pass it three times as
    /// `&dyn` of the three relevant ports — the helper is generic
    /// over `&dyn FeatureRepository` / `&dyn GateRepository`.
    #[allow(clippy::type_complexity)]
    fn make_adapter() -> (
        std::sync::Arc<SqliteAdapter>,
        std::sync::Arc<dyn ProjectRepository>,
        std::sync::Arc<dyn FeatureRepository>,
        std::sync::Arc<dyn GateRepository>,
    ) {
        let conn = Connection::open_in_memory().unwrap();
        let adapter = std::sync::Arc::new(SqliteAdapter::new(conn).unwrap());
        let projects: std::sync::Arc<dyn ProjectRepository> = adapter.clone();
        let features: std::sync::Arc<dyn FeatureRepository> = adapter.clone();
        let gates: std::sync::Arc<dyn GateRepository> = adapter.clone();
        (adapter, projects, features, gates)
    }

    /// Insert the parent `Project` and `Feature` rows that the
    /// `step_executions` foreign key requires. Returns the
    /// `FeatureId` used so callers can reuse it in the
    /// `StepExecution::feature_id` field.
    fn seed_parent_rows(
        projects: &dyn ProjectRepository,
        features: &dyn FeatureRepository,
    ) -> FeatureId {
        let now = crate::paths::now_ms();
        projects
            .add(crate::domain::models::Project {
                id: ProjectId::from("p-1".to_string()),
                name: "test".to_string(),
                compute_type: "local".to_string(),
                remote_host: None,
                status: "idle".to_string(),
                nodes: 0,
                spend: 0.0,
                tokens: 0,
                created_at: now,
            })
            .unwrap();
        features
            .add(Feature {
                id: FeatureId::from("f-1".to_string()),
                project_id: ProjectId::from("p-1".to_string()),
                workflow_id: Some(WorkflowId::from("w-1".to_string())),
                title: "test feature".to_string(),
                status: "running".to_string(),
                total_cost: 0.0,
                tokens: 0,
                duration: "0s".to_string(),
                agent_kind: None,
                model: None,
                mr_url: None,
                mr_state: Some("none".to_string()),
                created_at: now,
                commit_artifacts: None,
                loop_iterations: None,
                step_overrides: Vec::new(),
            })
            .unwrap();
        FeatureId::from("f-1".to_string())
    }

    /// Stand-in `StepExecution` builder. The helper only reads `id`
    /// and `step_id`, but the repo refuses garbage values for the
    /// other fields, so we fill in plausible ones.
    fn make_step_exec(id: &str, step_id: &str, index: u32, status: &str) -> StepExecution {
        let now = crate::paths::now_ms();
        StepExecution {
            id: StepExecutionId::from(id.to_string()),
            feature_id: FeatureId::from("f-1".to_string()),
            step_id: crate::domain::ids::StepId::from(step_id.to_string()),
            step_index: index,
            step_kind: "agent".to_string(),
            status: status.to_string(),
            cost_usd: Some(0.42),
            tokens: Some(1234),
            wall_clock_secs: Some(7),
            artifact_path: Some("artifacts/spec.md".to_string()),
            artifact_paths: vec!["artifacts/spec.md".to_string()],
            error_message: None,
            iteration_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_gate_exec(id: &str, index: u32) -> StepExecution {
        let now = crate::paths::now_ms();
        StepExecution {
            id: StepExecutionId::from(id.to_string()),
            feature_id: FeatureId::from("f-1".to_string()),
            step_id: crate::domain::ids::StepId::from("s-gate".to_string()),
            step_index: index,
            step_kind: "gate".to_string(),
            status: "awaiting_gate".to_string(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            artifact_path: None,
            artifact_paths: Vec::new(),
            error_message: None,
            iteration_count: 0,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn reset_marks_target_step_pending_and_clears_artifacts() {
        // Mirror the real bug: spec is `completed` with artifacts
        // attached, the gate is mid-decision. The helper must
        // rewind spec to `pending` and drop the artifacts so the
        // re-run starts from a clean slate.
        let (_adapter, projects, features, gates) = make_adapter();
        let _f_id = seed_parent_rows(&*projects, &*features);

        let spec = make_step_exec("se-spec", "s-spec", 1, "completed");
        let gate = make_gate_exec("se-gate", 2);
        let step_execs = vec![
            make_step_exec("se-research", "s-research", 0, "completed"),
            spec.clone(),
            gate.clone(),
        ];

        // Persist the spec + gate so the reset reads / writes hit
        // real rows. The research row is included so the step index
        // math lines up.
        features.step_create(step_execs[0].clone()).unwrap();
        features.step_create(spec.clone()).unwrap();
        features.step_create(gate.clone()).unwrap();

        // Pre-condition: spec carries the artifact from the
        // previous run, gate has an open decision.
        assert_eq!(
            features.step_get(&spec.id).unwrap().unwrap().status,
            "completed"
        );
        assert_eq!(
            features.step_get(&spec.id).unwrap().unwrap().artifact_paths,
            vec!["artifacts/spec.md".to_string()]
        );

        reset_for_redirect(&*features, &*gates, &step_execs, 1, &gate.id);

        // Post-condition: spec is pending with cleared counters
        // and dropped artifacts. The driver will now see the spec
        // as "not yet done" and re-run it instead of skipping past
        // it.
        let spec_after = features.step_get(&spec.id).unwrap().unwrap();
        assert_eq!(spec_after.status, "pending");
        assert_eq!(spec_after.artifact_path, None);
        assert!(spec_after.artifact_paths.is_empty());
        assert_eq!(spec_after.cost_usd, Some(0.0));
        assert_eq!(spec_after.tokens, Some(0));
        assert_eq!(spec_after.wall_clock_secs, Some(0));
    }

    #[test]
    fn reset_clears_gate_decision_row() {
        // The second half of the fix: the gate's own decision row
        // must be deleted, not just updated to `None`. After the
        // reset, the next visit to the gate must find no recorded
        // decision so the reconciliation falls through to the
        // in-process waiter (or the startup watchdog on a fresh
        // launch) and re-prompts the user.
        let (_adapter, projects, features, gates) = make_adapter();
        let _f_id = seed_parent_rows(&*projects, &*features);

        let gate = make_gate_exec("se-gate", 2);
        features.step_create(gate.clone()).unwrap();
        gates
            .create(GateDecision {
                id: GateDecisionId::from("gd-se-gate".to_string()),
                step_execution_id: gate.id.clone(),
                decision: Some("redirect".to_string()),
                feedback: Some("revise the spec to use cargo before mise".to_string()),
                created_at: crate::paths::now_ms(),
            })
            .unwrap();

        // Sanity check: the decision is in place.
        assert_eq!(
            gates
                .latest_for_step(&gate.id)
                .unwrap()
                .unwrap()
                .decision
                .as_deref(),
            Some("redirect")
        );

        let step_execs = vec![
            make_step_exec("se-spec", "s-spec", 1, "completed"),
            gate.clone(),
        ];
        reset_for_redirect(&*features, &*gates, &step_execs, 1, &gate.id);

        // The decision row is gone; `latest_for_step` returns None
        // and the gate's reconciliation will treat this as
        // "no decision yet, await user".
        assert!(gates.latest_for_step(&gate.id).unwrap().is_none());
    }

    #[test]
    fn reset_is_noop_when_target_index_out_of_bounds() {
        // Defensive: a misbehaving `resolve_redirect_target` could
        // return a stale index after the workflow shape changes
        // (e.g. the user re-ran `replay_from_step` and the indices
        // shifted). The helper must not panic; it just has nothing
        // to update. The gate decision is still cleared, since
        // that's an unconditional part of the redirect.
        let (_adapter, projects, features, gates) = make_adapter();
        let _f_id = seed_parent_rows(&*projects, &*features);

        let gate = make_gate_exec("se-gate", 2);
        features.step_create(gate.clone()).unwrap();
        gates
            .create(GateDecision {
                id: GateDecisionId::from("gd-se-gate".to_string()),
                step_execution_id: gate.id.clone(),
                decision: Some("redirect".to_string()),
                feedback: None,
                created_at: crate::paths::now_ms(),
            })
            .unwrap();

        let step_execs = vec![gate.clone()];
        reset_for_redirect(&*features, &*gates, &step_execs, 99, &gate.id);

        // The decision was still cleared; the out-of-bounds target
        // is silently skipped.
        assert!(gates.latest_for_step(&gate.id).unwrap().is_none());
    }
}
