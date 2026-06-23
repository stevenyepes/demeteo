use super::ExecutionDriver;
use crate::domain::models::{StepConfig, StepExecution};
use crate::ports::db::StepExecutionPatch;
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

        let max = step_conf.max_iterations.unwrap_or(0);
        let already = step_exec.iteration_count;
        if already + 1 > max {
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
                Some(format!(
                    "{} (retry budget exhausted: {} of {} attempts on '{}')",
                    msg, already, max, target_id.0
                )),
            );
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
