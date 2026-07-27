use std::time::Instant;

use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::db::{FeaturePatch, FeatureRepository, StepExecutionPatch};
use crate::ports::notification::{DomainEvent, NotificationPort};

/// Set a step execution to a final status (completed / failed / interrupted / awaiting_gate)
/// and emit the corresponding notification. Always sets cost_usd, tokens, wall_clock_secs
/// and the cache-token telemetry to the caller-provided values.
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn update_step_status(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    step_exec: &StepExecution,
    f_id: &FeatureId,
    status: &str,
    cost_usd: f64,
    tokens: Option<i64>,
    wall_clock_secs: u64,
    artifact_path: Option<String>,
    error_message: Option<String>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
) {
    let _ = try_update_step_status(
        features,
        notif,
        step_exec,
        f_id,
        status,
        cost_usd,
        tokens,
        wall_clock_secs,
        artifact_path,
        error_message,
        cache_read_input_tokens,
        cache_creation_input_tokens,
    );
}

/// [`update_step_status`], but surfacing whether the **durable write** landed.
///
/// Almost every caller is reporting a transition it has already committed to
/// and has nothing useful to do about a repository error, which is why the
/// infallible form above exists. The exception is a caller whose *control
/// flow* depends on the write being visible to the next read — the scheduler's
/// skip persistence, which re-derives its state from these rows and would
/// otherwise re-decide the same skip forever.
///
/// The event is still emitted only after a successful write (the P1.13
/// ordering: the row is the truth, the event is its push).
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_update_step_status(
    features: &dyn FeatureRepository,
    notif: &dyn NotificationPort,
    step_exec: &StepExecution,
    f_id: &FeatureId,
    status: &str,
    cost_usd: f64,
    tokens: Option<i64>,
    wall_clock_secs: u64,
    artifact_path: Option<String>,
    error_message: Option<String>,
    cache_read_input_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
) -> Result<(), String> {
    features.step_update(
        &step_exec.id,
        &StepExecutionPatch {
            last_failure_fingerprint: None,
            iteration_count: None,
            status: Some(status.to_string()),
            cost_usd: Some(Some(cost_usd)),
            tokens: Some(tokens),
            wall_clock_secs: Some(Some(wall_clock_secs)),
            artifact_path: artifact_path.map(Some),
            artifact_paths: None,
            error_message: error_message.map(Some),
            cache_read_input_tokens: Some(cache_read_input_tokens),
            cache_creation_input_tokens: Some(cache_creation_input_tokens),
        },
    )?;
    let _ = notif.emit(&DomainEvent::StepProgress {
        feature_id: f_id.clone(),
        step_id: step_exec.step_id.0.clone(),
        status: status.into(),
        cost_usd: Some(cost_usd),
        tokens,
        wall_clock_secs: Some(wall_clock_secs),
        cache_read_input_tokens,
        cache_creation_input_tokens,
    });
    Ok(())
}

/// Mark a feature as completed / failed / cancelled, summing step costs and tokens for total_cost/tokens.
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
    let total_tokens = features
        .steps_for_feature(f_id)
        .map(|list| list.iter().map(|s| s.tokens.unwrap_or(0)).sum::<i64>())
        .unwrap_or(0);
    let total_dur = format!("{}s", start_time.elapsed().as_secs());
    let _ = features.update(
        f_id,
        &FeaturePatch {
            status: Some(status.to_string()),
            total_cost: Some(Some(total_cost)),
            tokens: Some(Some(total_tokens)),
            duration: Some(Some(total_dur)),
            ..Default::default()
        },
    );
    let _ = notif.emit(&DomainEvent::FeatureStatusChanged {
        feature_id: f_id.clone(),
        status: status.into(),
    });
}
