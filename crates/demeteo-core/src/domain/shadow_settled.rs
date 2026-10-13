//! Whether a detached run's shadow has anything left to learn from its runner.
//!
//! The desktop stops asking the runner about a run once nothing there can
//! change, and "the run row is terminal" is not that. The run row and the
//! feature are written by different tasks on the runner: the row by whichever
//! task was waiting on the run, the feature by the engine's driver. A row can
//! go terminal while the driver is still working — a run parked `failed` by
//! the reboot-retry budget keeps its driver, and a terminal push that fails
//! marks the row before anyone looks at the feature again.
//!
//! Stopping on the row alone froze the shadow at whatever the last poll
//! copied: a feature shown `running` with its final step `running`, for a run
//! whose feature had long since reached `awaiting_mr`. Every action offered on
//! a finished feature — publishing it above all — keys off the shadow's
//! status, so the one recovery the desktop has was unreachable from it.

/// Run-row statuses no task on the runner will move.
const RUN_AT_REST: [&str; 4] = ["failed", "cancelled", "awaiting_mr", "completed"];

/// Feature statuses the engine's driver has let go of.
const FEATURE_AT_REST: [&str; 4] = ["failed", "cancelled", "awaiting_mr", "completed"];

/// `true` when polling the runner again cannot change the shadow.
///
/// `shadow_feature_status` is the local shadow row's status, `None` when the
/// run never got as far as a feature — then the row is all there is.
pub fn shadow_settled(run_status: &str, shadow_feature_status: Option<&str>) -> bool {
    RUN_AT_REST.contains(&run_status)
        && shadow_feature_status.is_none_or(|status| FEATURE_AT_REST.contains(&status))
}

#[cfg(test)]
#[path = "../../tests/domain/shadow_settled.rs"]
mod tests;
