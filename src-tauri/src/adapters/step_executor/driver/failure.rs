use super::ExecutionDriver;
use crate::domain::models::{Notification, NotificationKind, StepConfig, StepExecution};
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::DomainEvent;
use std::time::Instant;

impl ExecutionDriver {
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
        );
        super::super::updates::finish_feature(
            &*self.features,
            &*self.notif,
            &self.f_id,
            "failed",
            self.start_time,
        );
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
    }

    pub(crate) fn evaluate_on_failure(
        &self,
        step_exec: &StepExecution,
        step_conf: &StepConfig,
        msg: &str,
        accumulated_cost: f64,
        accumulated_tokens: i64,
        step_start: Instant,
    ) -> Option<usize> {
        let target_id = match step_conf.on_failure.as_ref() {
            Some(id) if !id.0.is_empty() => id,
            _ => return None,
        };

        let max = self.effective_loop_iterations(step_conf);
        let already = step_exec.iteration_count;
        if already + 1 > max {
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
            return None;
        }

        let target_idx = self.steps.iter().position(|s| s.id == *target_id)?;
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
                msg,
                target_id.0,
                already + 1,
                max
            )),
        );

        let _ = self.features.step_update(
            &step_exec.id,
            &StepExecutionPatch {
                iteration_count: Some(already + 1),
                ..Default::default()
            },
        );

        Some(target_idx)
    }
}
