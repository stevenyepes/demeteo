use crate::domain::intercept::{ExecutionResult, InterceptPayload};

pub const EVENT_PERMISSION_REQUESTED: &str = "permission_requested";
pub const EVENT_COMMAND_EXECUTED: &str = "command_executed";

pub const EVENT_FEATURE_STATUS_CHANGED: &str = "feature_status_changed";
pub const EVENT_STEP_PROGRESS: &str = "step_progress";
pub const EVENT_GATE_REQUIRED: &str = "gate_required";
pub const EVENT_CONFLICT_DETECTED: &str = "conflict_detected";

pub trait NotificationPort: Send + Sync {
    fn emit_permission_requested(
        &self,
        payload: &InterceptPayload,
    ) -> Result<(), String>;

    fn emit_command_executed(
        &self,
        thread_id: &str,
        machine_id: &str,
        result: &ExecutionResult,
        intercept_id: Option<&str>,
    ) -> Result<(), String>;

    fn emit_feature_status_changed(&self, feature_id: &str, status: &str) -> Result<(), String>;
    fn emit_step_progress(
        &self,
        feature_id: &str,
        step_id: &str,
        status: &str,
        cost_usd: Option<f64>,
        wall_clock_secs: Option<u64>,
    ) -> Result<(), String>;
    fn emit_gate_required(&self, feature_id: &str, step_execution_id: &str) -> Result<(), String>;
    fn emit_conflict_detected(&self, feature_id: &str, subtask_id: &str) -> Result<(), String>;
}
