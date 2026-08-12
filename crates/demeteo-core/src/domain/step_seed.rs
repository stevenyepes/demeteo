//! The step rows a run is registered with.
//!
//! A free function over domain types alone, so what a freshly-seeded row
//! carries is answerable from a test. Its call site is the `registering` phase
//! of the bootstrap tail, wedged between an origin sync and a driver spawn —
//! reaching it there means a `DagStepExecutor` with every port those
//! neighbouring phases touch, which is why none of this was asserted anywhere.

use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::models::{StepConfig, StepExecution};

/// Whether `step_execution_id` names one of `feature_id`'s rows.
///
/// A driver shares its in-memory registries — the gate waiters above all —
/// with every other driver in the process, and they are keyed by
/// step-execution id. So a run tearing itself down is holding another run's
/// keys, and the difference between sweeping its own and sweeping the map is
/// the difference between a leak and a wedge: clearing a live waiter leaves
/// that driver parked on a rendezvous nobody can reach, its `gate_decide`
/// recording decisions that nothing applies and `ensure_driver_running`
/// declining to help because the driver is, technically, alive.
///
/// Spelled here rather than at the sweep so it sits against the `format!`
/// below that derives the id; a prefix rule kept anywhere else is a rule that
/// drifts the day the id gains a field.
pub fn belongs_to_feature(feature_id: &FeatureId, step_execution_id: &str) -> bool {
    step_execution_id.starts_with(&format!("se-{}-", feature_id.as_str()))
}

/// One `pending` row per configured step, `step_index` following the slice
/// order, spend measured at zero rather than unknown.
///
/// The id is derived from the pair — `se-<feature>-<step>` — rather than
/// minted, so the same feature seeded twice names the same rows rather than a
/// second set.
pub fn seed_step_executions(
    feature_id: &FeatureId,
    steps: &[StepConfig],
    now: i64,
) -> Vec<StepExecution> {
    steps
        .iter()
        .enumerate()
        .map(|(i, step)| StepExecution {
            id: StepExecutionId::from(format!("se-{}-{}", feature_id.as_str(), step.id.0)),
            feature_id: feature_id.clone(),
            step_id: step.id.clone(),
            step_index: i as u32,
            step_kind: step.kind.clone(),
            status: "pending".to_string(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            artifact_path: None,
            artifact_paths: Vec::new(),
            error_message: None,
            iteration_count: 0,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            last_failure_fingerprint: None,
            created_at: now,
            updated_at: now,
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/step_seed.rs"]
mod tests;
