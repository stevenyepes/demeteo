use std::time::Instant;

use crate::adapters::step_executor::driver::{ExecutionDriver, RetryContext};
use crate::adapters::step_executor::gate_waiter::GateWaiter;
use crate::adapters::step_executor::steps::StepOutcome;
use crate::domain::ids::{FeatureId, GateDecisionId, StepId};
use crate::domain::models::{GateDecision, StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::{DomainEvent, NotificationPort};

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
///   1. Step ID in `feedback` (if it matches one of `steps`) — either the
///      whole trimmed feedback, or a whole word within a longer free-text
///      note (e.g. "redo s-tickets, the split is too coarse"). A pipeline
///      can have more than one artifact-only predecessor ahead of a gate
///      (e.g. ticket decomposition followed by a spec step); a reviewer who
///      names the one they mean should land there even without typing
///      nothing else, rather than falling through to a fallback that may
///      guess the other one.
///   2. `on_failure` on the gate's step config.
///   3. The nearest preceding step whose effective capability is
///      `Implement`. This is the natural intent of "give the agent
///      my feedback and redo it" — implementation feedback should
///      land on a step that can actually modify code. Without this
///      rule, feedback at `s-gate-ship` (index 6 in the standard
///      pipeline) routes to `s-validate` (index 5), which is a
///      verify-only step that documents findings but cannot write
///      code, so the user's feedback just gets logged into
///      `validation-report.md` and bounced back to `s-implement`
///      via the verifier two iterations later.
///   4. The step immediately before the gate — a safety net for
///      workflows that have no implement-capable step preceding
///      the gate (e.g. a pre-implementation review gate). Keeps
///      the pipeline from silently cancelling on free-text feedback.
///   5. `None` only when the gate is the very first step.
fn resolve_redirect_target(
    steps: &[StepConfig],
    on_failure: Option<&StepId>,
    gate_step_index: u32,
    feedback: Option<&str>,
) -> Option<usize> {
    let explicit = feedback
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|cleaned| {
            steps.iter().position(|s| s.id.0 == cleaned).or_else(|| {
                // Whole-word search: a bare substring match would also fire
                // on "s-tickets2" or a step id that is a prefix of another,
                // so split on anything that isn't part of a kebab-case id.
                steps.iter().position(|s| {
                    cleaned
                        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                        .any(|token| token == s.id.0)
                })
            })
        });

    let implement_fallback = |gate_idx: usize| -> Option<usize> {
        if gate_idx == 0 {
            return None;
        }
        steps[..gate_idx].iter().rposition(|s| {
            s.effective_capability() == crate::domain::permission::StepCapability::Implement
        })
    };

    let predecessor_fallback = |gate_idx: u32| -> Option<usize> {
        if gate_idx > 0 {
            Some(gate_idx as usize - 1)
        } else {
            None
        }
    };

    explicit
        .or_else(|| on_failure.and_then(|id| steps.iter().position(|s| s.id == *id)))
        .or_else(|| implement_fallback(gate_step_index as usize))
        .or_else(|| predecessor_fallback(gate_step_index))
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
///     skip past it;
///   * the gate's own status row is flipped from `awaiting_gate`
///     back to `pending` so the timeline stops displaying the
///     "Decide Gate" affordance while the redirected step is
///     re-running (the gate will re-emit `awaiting_gate` on its
///     next visit); and
///   * the gate's own `gate_decisions` row is cleared so the next
///     visit to the gate re-prompts the user. Without this third
///     half, the gate's reconciliation would find the prior
///     `redirect` decision on file, return
///     `RedirectTo(target_idx)` once more, and the same step would
///     loop forever — the bug this fix exists to break.
///
/// Each DB mutation is paired with a `StepProgress` event so the
/// frontend's local `steps` array picks up the new status without
/// waiting for a full `step_list_for_run` poll. Missing the event
/// leaves the timeline showing "Decide Gate" / "Retry Step" for
/// rows whose DB state has already moved on (the bug this fix
/// exists to break in the UI layer).
///
/// All writes are best-effort. Failures are intentionally
/// swallowed: the redirect already won the user's intent, and any
/// stale state is recoverable on the next reconciliation pass
/// (the startup watchdog will re-surface the gate if the driver
/// dies between the reset and the target step completing).
#[allow(clippy::too_many_arguments)]
fn reset_for_redirect(
    features: &dyn crate::ports::db::FeatureRepository,
    gates: &dyn crate::ports::db::GateRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
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
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("pending".to_string()),
                cost_usd: Some(Some(0.0)),
                tokens: Some(Some(0)),
                wall_clock_secs: Some(Some(0)),
                artifact_path: Some(None),
                artifact_paths: Some(Vec::new()),
                error_message: Some(None),
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = notif.emit(&DomainEvent::StepProgress {
            feature_id: f_id.clone(),
            step_id: target_exec.step_id.0.clone(),
            status: "pending".into(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
    }
    // Clear this gate's own decision row so the next visit to the
    // gate re-prompts the user. Idempotent against app restarts: if
    // the driver dies after the reset and before the target step
    // finishes, the startup watchdog will already mark the gate
    // `interrupted` and create a fresh `gate_decisions` row with
    // `decision = None` (see `startup_watchdog` in
    // `impl_traits/mod.rs`).
    let _ = gates.reset_for_step_execution(gate_step_execution_id);
    // Flip the gate's own status from `awaiting_gate` to `pending`
    // so the timeline stops showing the "Decide Gate" button while
    // the redirected step re-runs. Without this update the gate
    // remains `awaiting_gate` in the DB and the frontend's stale
    // local cache keeps rendering the decision affordance — even
    // though the user already submitted a decision and the gate
    // won't re-prompt until the target finishes. Fetch the row
    // first so we have the gate's `step_id` to put in the event.
    if let Ok(Some(gate_exec)) = features.step_get(gate_step_execution_id) {
        let _ = features.step_update(
            gate_step_execution_id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: None,
                status: Some("pending".to_string()),
                cost_usd: None,
                tokens: None,
                wall_clock_secs: None,
                artifact_path: None,
                artifact_paths: None,
                error_message: None,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
            },
        );
        let _ = notif.emit(&DomainEvent::StepProgress {
            feature_id: f_id.clone(),
            step_id: gate_exec.step_id.0.clone(),
            status: "pending".into(),
            cost_usd: None,
            tokens: None,
            wall_clock_secs: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        });
    }
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
            // `reject` is the remote inbox's word for `cancel` (the
            // detached-run gate buttons are Approve / Reject). Spelled out
            // rather than left to the catch-all below, which cancels on
            // *any* unrecognised decision — so a genuine typo stays
            // distinguishable from a rejection.
            Some("cancel") | Some("reject") => StepOutcome::Failed("Gate Cancelled".to_string()),
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
                        reset_for_redirect(
                            &*self.features,
                            &*self.gates,
                            &*self.notif,
                            &self.f_id,
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
#[path = "../../../../tests/infrastructure/step_executor/gate_redirect_target.rs"]
mod redirect_target_tests;

/// The bug this regression suite exists to break: when a gate
/// redirects back to a previous step with feedback, the orchestrator
/// used to re-run the target step, then re-enter the gate, find the
/// same `redirect` decision on file, redirect back again — and loop
/// forever. `reset_for_redirect` is the fix: it resets the target
/// step's status to `pending` and clears the gate's own decision
/// row. These tests pin both halves of the fix in place.
#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/gate_redirect_reset.rs"]
mod redirect_reset_tests;

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
