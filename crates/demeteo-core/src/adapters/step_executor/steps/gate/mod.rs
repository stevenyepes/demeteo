use std::time::Instant;

use crate::adapters::step_executor::driver::{ExecutionDriver, RetryContext};
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::gate::decision::{classify, GateVerdict};
use crate::domain::gate::redirect::resolve_redirect_target;
use crate::domain::ids::GateDecisionId;
use crate::domain::models::{GateDecision, StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;

mod redirect_reset;

use redirect_reset::{reset_gate_target, GateWriters, RedirectReset};

/// What the gate inherited and what the run has spent reaching it —
/// everything [`ExecutionDriver::apply_gate_decision`] needs that is not the
/// decision itself.
///
/// A gate produces nothing of its own: it carries its predecessor's artifacts
/// onto its own row so the review card has something to show, and carries the
/// run's accumulated cost onto whichever terminal status it lands on. Those
/// two carries are the concept; the fields exist because a decision cannot be
/// applied without them.
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
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("awaiting_gate".to_string()),
                cost_usd: Some(Some(*accumulated_cost)),
                tokens: None,
                wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                artifact_path: prev_artifact_path.as_ref().map(|p| Some(p.clone())),
                artifact_paths: Some(prev_artifact_paths.clone()),
                error_message: Some(None),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
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

        // A human gate can park the run for hours. Anthropic's prompt cache
        // has a ~5-minute TTL, so `--resume`ing a session after the gate
        // would replay the entire accumulated transcript at full input
        // price — strictly worse than a fresh session that re-warms the
        // shared static prefix. Kill everything now; the next agent step
        // spawns fresh. (The reconciliation fast-path above skips this —
        // a decision that already arrived means no idle gap.)
        self.registry.kill_all_for_feature(self.f_id.as_str()).await;

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
        let step_ids: Vec<crate::domain::ids::StepId> =
            self.steps.iter().map(|s| s.id.clone()).collect();
        match classify(
            decision_recvd.decision.as_deref(),
            decision_recvd.feedback.as_deref(),
            &step_ids,
        ) {
            GateVerdict::Approve { signal } => {
                if let Some(cleaned) = signal {
                    self.capture_signal(
                        Some(ctx.step_exec.id.0.clone()),
                        crate::domain::memory::SignalKind::GateFeedback,
                        format!(
                            "Gate '{}' approved with feedback: {}",
                            ctx.step_exec.step_id.0, cleaned
                        ),
                    );
                }

                let wall = ctx.step_start.elapsed().as_secs();
                let _ = self.features.step_update(
                    &ctx.step_exec.id,
                    &StepExecutionPatch {
                        last_failure_fingerprint: None,
                        iteration_count: None,
                        status: Some("completed".to_string()),
                        cost_usd: Some(Some(*ctx.accumulated_cost)),
                        tokens: None,
                        wall_clock_secs: Some(wall).map(|_v| Some(wall)),
                        artifact_path: ctx.prev_artifact_path.as_ref().map(|p| Some(p.clone())),
                        artifact_paths: Some(ctx.prev_artifact_paths.to_vec()),
                        error_message: Some(None),
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
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
            GateVerdict::Cancel => StepOutcome::Failed("Gate Cancelled".to_string()),
            GateVerdict::Redirect {
                signal,
                retry_feedback,
            } => {
                // If feedback isn't a step ID, capture it as a memory
                // signal so the next attempt has the user's guidance.
                // Capture runs *before* target resolution so a non-id
                // feedback doesn't get silently lost.
                if let Some(cleaned) = signal {
                    self.capture_signal(
                        Some(ctx.step_exec.id.0.clone()),
                        crate::domain::memory::SignalKind::GateFeedback,
                        format!(
                            "Gate '{}' redirected with instruction: {}",
                            ctx.step_exec.step_id.0, cleaned
                        ),
                    );
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
                if let Some(cleaned) = retry_feedback {
                    self.retry_ctx = Some(RetryContext {
                        feedback: cleaned,
                        iteration: 1,
                        max: 1,
                        failing_tests: Vec::new(),
                        implicated_files: Vec::new(),
                        // The user's guidance stays visible to every step
                        // between the redirect target and this gate; it is
                        // cleared when the gate itself completes (i.e. the
                        // user approves the redone work).
                        failing_step_id: ctx.step_exec.step_id.0.clone(),
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
                        reset_gate_target(
                            GateWriters {
                                features: &*self.features,
                                gates: &*self.gates,
                                notif: &*self.notif,
                            },
                            &self.f_id,
                            RedirectReset {
                                step_execs: ctx.step_execs,
                                target_idx: idx,
                                gate_step_execution_id: &ctx.step_exec.id,
                            },
                        );
                        StepOutcome::RedirectTo(idx)
                    }
                    None => StepOutcome::Cancelled,
                }
            }
            GateVerdict::Unrecognised => StepOutcome::Cancelled,
        }
    }
}

// ── NodeHandler registration (P1.7) ───────────────────────────────────────────

/// The `gate` node type behind the [`NodeHandler`] seam. Pure
/// delegation: execution is [`ExecutionDriver::handle_gate_step`],
/// byte-for-byte the behavior the old `match` arm dispatched (the arm's
/// defensive `step_conf` clone is obsolete here — the dispatch loop
/// already hands the registry a clone that doesn't borrow the driver).
///
/// [`NodeHandler`]: crate::adapters::step_executor::registry::NodeHandler
pub(crate) struct GateNodeHandler;

/// JSON Schema for the `gate` node's `config` payload. A gate is a
/// durable suspend point — the only HITL surface (Decision 35) — so
/// its config is thin: the blast-radius class that decides whether an
/// unattended run may auto-approve it.
#[allow(dead_code)] // Read via `NodeHandler::config_schema` (first runtime caller: P3.1).
static GATE_CONFIG_SCHEMA: std::sync::LazyLock<serde_json::Value> =
    std::sync::LazyLock::new(|| {
        serde_json::json!({
            "type": "object",
            "description": "Configuration for a `gate` node: park the run \
                for a human decision (approve / reject / redirect). The \
                gate inherits its predecessor's artifacts for review.",
            "properties": {
                "gate_class": {
                    "type": ["string", "null"],
                    "enum": ["dangerous", "safe", null],
                    "description": "Blast-radius class. `dangerous` (merge \
                        to default, push to protected, deploy, delete) is \
                        parked for a human even on unattended runs; unset \
                        or `safe` auto-approves unattended."
                }
            },
            "additionalProperties": true
        })
    });

#[async_trait::async_trait]
impl crate::adapters::step_executor::registry::NodeHandler for GateNodeHandler {
    fn kind(&self) -> &'static str {
        "gate"
    }

    fn config_schema(&self) -> &'static serde_json::Value {
        &GATE_CONFIG_SCHEMA
    }

    fn display(&self) -> crate::adapters::step_executor::registry::NodeDisplay {
        crate::adapters::step_executor::registry::NodeDisplay {
            label: "Gate",
            summary: "Pause for a human decision: approve, reject, or send the \
                      run back to an earlier node with feedback.",
        }
    }

    fn ports(&self) -> crate::adapters::step_executor::registry::NodePorts {
        use crate::domain::models::workflow_v2::PortType;
        crate::adapters::step_executor::registry::NodePorts {
            inputs: &[PortType::Any],
            outputs: &[PortType::Approval],
        }
    }

    async fn execute(
        &self,
        ctx: crate::adapters::step_executor::registry::NodeCtx<'_>,
    ) -> StepOutcome {
        ctx.driver
            .handle_gate_step(
                ctx.step_exec,
                ctx.step_conf,
                ctx.accumulated_cost,
                ctx.step_start,
                ctx.step_index,
                ctx.step_execs,
            )
            .await
    }
}
