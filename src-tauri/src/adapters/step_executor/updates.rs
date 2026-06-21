use std::time::Instant;

use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};
use crate::ports::notification::{DomainEvent, NotificationPort};

/// Set a step execution to a final status (completed / failed / interrupted / awaiting_gate)
/// and emit the corresponding notification. Always sets cost_usd and wall_clock_secs to the
/// caller-provided values.
#[allow(dead_code)]
pub(crate) fn update_step_status(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    step_exec: &StepExecution,
    f_id: &FeatureId,
    status: &str,
    cost_usd: f64,
    wall_clock_secs: u64,
    artifact_path: Option<String>,
    error_message: Option<String>,
) {
    let _ = features.step_update(
        &step_exec.id,
        &StepExecutionPatch {
            iteration_count: None,
            status: Some(status.to_string()),
            cost_usd: Some(Some(cost_usd)),
            wall_clock_secs: Some(Some(wall_clock_secs)),
            artifact_path: artifact_path.map(|p| Some(p)),
            artifact_paths: None,
            error_message: error_message.map(|msg| Some(msg)),
        },
    );
    let _ = notif.emit(&DomainEvent::StepProgress {
        feature_id: f_id.clone(),
        step_id: step_exec.step_id.0.clone(),
        status: status.into(),
        cost_usd: Some(cost_usd),
        wall_clock_secs: Some(wall_clock_secs),
    });
}

/// Mark a feature as completed / failed / cancelled, summing step costs for total_cost.
#[allow(dead_code)]
pub(crate) fn finish_feature(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    f_id: &FeatureId,
    status: &str,
    start_time: Instant,
) {
    let total_cost = features
        .steps_for_feature(f_id)
        .map(|list| list.iter().map(|s| s.cost_usd.unwrap_or(0.0)).sum::<f64>())
        .unwrap_or(0.0);
    let total_dur = format!("{}s", start_time.elapsed().as_secs());
    let _ = features.update(
        f_id,
        &FeaturePatch {
            status: Some(status.to_string()),
            total_cost: Some(Some(total_cost)),
            duration: Some(Some(total_dur)),
            ..Default::default()
        },
    );
    let _ = notif.emit(&DomainEvent::FeatureStatusChanged {
        feature_id: f_id.clone(),
        status: status.into(),
    });
}
