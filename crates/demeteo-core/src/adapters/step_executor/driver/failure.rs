use super::ExecutionDriver;
use crate::adapters::step_executor::retry_policy::{RetryAction, RetryDecision};
use crate::domain::ids::StepId;
use crate::domain::models::{Notification, NotificationKind, StepExecution};
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;
use std::time::Instant;

impl ExecutionDriver {
    /// Narrate one retry-policy decision (P1.13): every failure names the
    /// rule that answered it in the live event stream, mirroring what the
    /// attempt row already stores in `applied_rule`. Emitted *before* the
    /// decision is acted on, so the log reads decision-then-consequence.
    pub(crate) fn emit_retry_decision(
        &self,
        step_exec: &StepExecution,
        decision: &RetryDecision,
        reason: &str,
    ) {
        let (action, target_id) = match &decision.action {
            RetryAction::Redirect { target, .. } => ("redirect", Some(target.0.clone())),
            RetryAction::Exhausted { target } => {
                ("exhausted", target.as_ref().map(|t| t.0.clone()))
            }
            RetryAction::RetryInPlace { .. } => ("in_place", None),
            RetryAction::Fail => ("fail", None),
        };
        // `rule_id` is `<class>.<strategy>` (retry_policy::evaluate) and
        // class names contain no dot, so the prefix *is* the class.
        let error_class = decision
            .rule_id
            .split('.')
            .next()
            .unwrap_or_default()
            .to_string();
        let _ = self.notif.emit(&DomainEvent::RetryDecision {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            error_class,
            rule_id: decision.rule_id.clone(),
            action: action.to_string(),
            target_id,
            attempt: decision.attempt,
            max: decision.max_attempts,
            reason: reason.to_string(),
        });
    }
    pub(crate) async fn fail_step_and_feature(
        &self,
        step_exec: &StepExecution,
        msg: &str,
        accumulated_cost: f64,
        accumulated_tokens: i64,
        step_start: Instant,
    ) {
        let wall = step_start.elapsed().as_secs();
        super::super::updates::update_step_status(
            &*self.features,
            &*self.notif,
            step_exec,
            &self.f_id,
            "failed",
            accumulated_cost,
            Some(accumulated_tokens),
            wall,
            None,
            Some(msg.to_string()),
            self.last_cache_read,
            self.last_cache_creation,
        );
        super::super::updates::finish_feature(
            &*self.features,
            &*self.notif,
            &self.f_id,
            "failed",
            self.start_time,
        );
        // Sweep every fingerprint-scoped session this feature touched.
        // `handle_agent_step` only ever kills the *current* step's
        // session on failure; an earlier successful step with a
        // different permission-profile/model fingerprint (see
        // `ExecutionDriver::agent_session_key`) would otherwise be
        // left alive with nothing left to resume it.
        self.registry.kill_all_for_feature(self.f_id.as_str()).await;
        self.capture_signal(
            Some(step_exec.id.0.clone()),
            crate::domain::memory::SignalKind::Failure,
            format!("Step '{}' failed: {}", step_exec.step_id.0, msg),
        );
    }

    pub(crate) async fn cancel_feature(&self) {
        super::super::updates::finish_feature(
            &*self.features,
            &*self.notif,
            &self.f_id,
            "cancelled",
            self.start_time,
        );
        self.registry.kill_all_for_feature(self.f_id.as_str()).await;
    }

    /// Record an exhausted retry budget (a `redirect` rule with no
    /// attempts left, P1.10): the decorated step-status write, the
    /// persisted bell notification, and the live `RetryBudgetExhausted`
    /// event. The caller follows up with
    /// [`fail_step_and_feature`](Self::fail_step_and_feature) — same
    /// sequence the v1 engine produced.
    ///
    /// `already` is the number of retry attempts consumed; `max` the
    /// rule's effective budget.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_retry_exhausted(
        &self,
        step_exec: &StepExecution,
        target_id: &StepId,
        msg: &str,
        already: u32,
        max: u32,
        accumulated_cost: f64,
        accumulated_tokens: i64,
        step_start: Instant,
    ) {
        tracing::warn!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            target_step = %target_id.0,
            attempt = already,
            max,
            "retry budget exhausted"
        );
        let wall = step_start.elapsed().as_secs();
        let final_msg = format!(
            "{} (retry budget exhausted: {} of {} attempts on '{}')",
            msg, already, max, target_id.0
        );
        super::super::updates::update_step_status(
            &*self.features,
            &*self.notif,
            step_exec,
            &self.f_id,
            "failed",
            accumulated_cost,
            Some(accumulated_tokens),
            wall,
            None,
            Some(final_msg.clone()),
            self.last_cache_read,
            self.last_cache_creation,
        );
        // Persist a `notifications` row so the user sees the
        // signal in the bell after a refresh, mirroring how
        // `MrMerged` is persisted by `mr_monitor`. A failed
        // feature lookup is non-fatal: the live event below
        // still drives the toast.
        if let Ok(Some(feature)) = self.features.get(&self.f_id) {
            let notification = Notification {
                id: format!("notif-{}", crate::paths::now_ms()),
                project_id: feature.project_id.0.clone(),
                feature_id: self.f_id.0.clone(),
                kind: NotificationKind::RetryBudgetExhausted,
                message: format!(
                    "Step '{}' failed after {} attempt(s) — the agent couldn't fix it. Your turn.",
                    step_exec.step_id.0, already
                ),
                feature_url: Some(format!(
                    "/projects/{}/features/{}",
                    feature.project_id.0, self.f_id.0
                )),
                read: false,
                created_at: crate::paths::now_ms(),
            };
            let _ = self.notifications.add(notification);
        }
        // Push the live event so the toast reacts without
        // waiting for the user to refresh.
        let _ = self.notif.emit(&DomainEvent::RetryBudgetExhausted {
            feature_id: self.f_id.clone(),
            step_id: step_exec.step_id.0.clone(),
            target_id: target_id.0.clone(),
            attempt: already,
            max,
            reason: final_msg,
        });
    }

    /// Start a policy-granted redirect retry (P1.10): resolve the target
    /// node's index, record the "retrying: will jump" status, and bump
    /// the persisted `iteration_count`. Returns `None` — with **no**
    /// writes, matching the v1 dangling-`on_failure` behavior — when the
    /// target doesn't exist in this run's steps; the caller then fails
    /// the feature.
    ///
    /// `attempt` is the 1-based attempt now starting; `max` its budget.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn begin_redirect(
        &self,
        step_exec: &StepExecution,
        target_id: &StepId,
        msg: &str,
        attempt: u32,
        max: u32,
        accumulated_cost: f64,
        accumulated_tokens: i64,
        step_start: Instant,
    ) -> Option<usize> {
        let target_idx = self.steps.iter().position(|s| s.id == *target_id)?;
        tracing::info!(
            feature_id = %self.f_id,
            step_id = %step_exec.step_id.0,
            target_step = %target_id.0,
            attempt,
            max,
            "step retry → redirecting"
        );
        super::super::updates::update_step_status(
            &*self.features,
            &*self.notif,
            step_exec,
            &self.f_id,
            "failed",
            accumulated_cost,
            Some(accumulated_tokens),
            step_start.elapsed().as_secs(),
            None,
            Some(format!(
                "{} (retrying: will jump to '{}' on attempt {} of {})",
                msg, target_id.0, attempt, max
            )),
            self.last_cache_read,
            self.last_cache_creation,
        );

        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                last_failure_fingerprint: None,
                iteration_count: Some(attempt),
                ..Default::default()
            },
        );

        Some(target_idx)
    }

    /// Start a policy-granted in-place retry (P1.10; historically the
    /// environment one-shot): capture the retry signal and park the step
    /// back at `pending` so the run loop re-dispatches the same index.
    pub(crate) fn begin_in_place_retry(
        &self,
        step_exec: &StepExecution,
        msg: &str,
        accumulated_cost: f64,
        accumulated_tokens: i64,
        step_start: Instant,
    ) {
        self.capture_signal(
            Some(step_exec.id.0.clone()),
            crate::domain::memory::SignalKind::Retry,
            format!(
                "Step '{}' hit an environmental failure, retrying in place: {}",
                step_exec.step_id.0, msg
            ),
        );
        super::super::updates::update_step_status(
            &*self.features,
            &*self.notif,
            step_exec,
            &self.f_id,
            "pending",
            accumulated_cost,
            Some(accumulated_tokens),
            step_start.elapsed().as_secs(),
            None,
            Some(format!(
                "{} (environment issue — retrying step in place)",
                msg
            )),
            self.last_cache_read,
            self.last_cache_creation,
        );
    }
}
