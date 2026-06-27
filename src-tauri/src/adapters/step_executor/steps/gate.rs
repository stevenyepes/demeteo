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
                    Some(idx) => StepOutcome::RedirectTo(idx),
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
