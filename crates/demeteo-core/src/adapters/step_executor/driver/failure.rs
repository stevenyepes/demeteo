use super::ExecutionDriver;
use crate::adapters::step_executor::retry_policy::{RetryAction, RetryDecision};
use crate::adapters::step_executor::spend::StepSpend;
use crate::adapters::step_executor::step_status::{
    update_step_status, StatusWriters, StepTransition,
};
use crate::domain::ids::StepId;
use crate::domain::models::{Notification, NotificationKind, StepConfig, StepExecution};
use crate::ports::db::{NotificationRepository, StepExecutionPatch};
use crate::ports::notification::DomainEvent;

/// How far into its allowance one retry decision is.
///
/// The two numbers are meaningless apart — "attempt 3" says nothing
/// without "of 5" — and both the exhausted message and the redirect
/// message render them as exactly that pair.
///
/// `attempt` is the number the message names, which the two callers read
/// one decision apart: [`begin_redirect`] passes the attempt now
/// starting, [`record_retry_exhausted`] the last one that ran.
#[derive(Clone, Copy)]
pub(crate) struct RetryBudget {
    pub attempt: u32,
    pub max: u32,
}

/// Record an exhausted retry budget (a `redirect` rule with no
/// attempts left, P1.10): the decorated step-status write, the
/// persisted bell notification, and the live `RetryBudgetExhausted`
/// event. The caller follows up with
/// [`ExecutionDriver::fail_step_and_feature`] — same sequence the v1
/// engine produced.
pub(crate) fn record_retry_exhausted(
    writers: StatusWriters<'_>,
    notifications: &dyn NotificationRepository,
    step_exec: &StepExecution,
    target_id: &StepId,
    msg: &str,
    budget: RetryBudget,
    spend: StepSpend,
) {
    let RetryBudget {
        attempt: already,
        max,
    } = budget;
    tracing::warn!(
        feature_id = %writers.f_id,
        step_id = %step_exec.step_id.0,
        target_step = %target_id.0,
        attempt = already,
        max,
        "retry budget exhausted"
    );
    let final_msg = format!(
        "{} (retry budget exhausted: {} of {} attempts on '{}')",
        msg, already, max, target_id.0
    );
    update_step_status(
        writers,
        step_exec,
        StepTransition::failed(
            spend.cost,
            Some(spend.tokens),
            spend.wall_secs(),
            final_msg.clone(),
            spend.cache,
        ),
    );
    // Persist a `notifications` row so the user sees the
    // signal in the bell after a refresh, mirroring how
    // `MrMerged` is persisted by `mr_monitor`. A failed
    // feature lookup is non-fatal: the live event below
    // still drives the toast.
    if let Ok(Some(feature)) = writers.features.get(writers.f_id) {
        let notification = Notification {
            id: format!("notif-{}", crate::paths::now_ms()),
            project_id: feature.project_id.0.clone(),
            feature_id: writers.f_id.0.clone(),
            kind: NotificationKind::RetryBudgetExhausted,
            message: format!(
                "Step '{}' failed after {} attempt(s) — the agent couldn't fix it. Your turn.",
                step_exec.step_id.0, already
            ),
            feature_url: Some(format!(
                "/projects/{}/features/{}",
                feature.project_id.0, writers.f_id.0
            )),
            read: false,
            created_at: crate::paths::now_ms(),
        };
        let _ = notifications.add(notification);
    }
    // Push the live event so the toast reacts without
    // waiting for the user to refresh.
    let _ = writers.notif.emit(&DomainEvent::RetryBudgetExhausted {
        feature_id: writers.f_id.clone(),
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
/// target doesn't exist in `steps`; the caller then fails the feature.
pub(crate) fn begin_redirect(
    writers: StatusWriters<'_>,
    steps: &[StepConfig],
    step_exec: &StepExecution,
    target_id: &StepId,
    msg: &str,
    budget: RetryBudget,
    spend: StepSpend,
) -> Option<usize> {
    let RetryBudget { attempt, max } = budget;
    let target_idx = steps.iter().position(|s| s.id == *target_id)?;
    tracing::info!(
        feature_id = %writers.f_id,
        step_id = %step_exec.step_id.0,
        target_step = %target_id.0,
        attempt,
        max,
        "step retry → redirecting"
    );
    update_step_status(
        writers,
        step_exec,
        StepTransition::failed(
            spend.cost,
            Some(spend.tokens),
            spend.wall_secs(),
            format!(
                "{} (retrying: will jump to '{}' on attempt {} of {})",
                msg, target_id.0, attempt, max
            ),
            spend.cache,
        ),
    );

    let _ = writers.features.step_update(
        &step_exec.id,
        &StepExecutionPatch {
            last_failure_fingerprint: None,
            iteration_count: Some(attempt),
            ..Default::default()
        },
    );

    Some(target_idx)
}

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
        spend: StepSpend,
    ) {
        update_step_status(
            self.status_writers(),
            step_exec,
            StepTransition::failed(
                spend.cost,
                Some(spend.tokens),
                spend.wall_secs(),
                msg.to_string(),
                spend.cache,
            ),
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

    /// Start a policy-granted in-place retry (P1.10; historically the
    /// environment one-shot): capture the retry signal and park the step
    /// back at `pending` so the run loop re-dispatches the same index.
    pub(crate) fn begin_in_place_retry(
        &self,
        step_exec: &StepExecution,
        msg: &str,
        spend: StepSpend,
    ) {
        self.capture_signal(
            Some(step_exec.id.0.clone()),
            crate::domain::memory::SignalKind::Retry,
            format!(
                "Step '{}' hit an environmental failure, retrying in place: {}",
                step_exec.step_id.0, msg
            ),
        );
        update_step_status(
            self.status_writers(),
            step_exec,
            StepTransition::pending(
                spend.cost,
                spend.tokens,
                spend.wall_secs(),
                format!("{} (environment issue — retrying step in place)", msg),
                spend.cache,
            ),
        );
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/driver/retry_writes.rs"]
mod retry_writes_tests;
