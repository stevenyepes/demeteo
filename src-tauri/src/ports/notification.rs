use crate::domain::intercept::{ExecutionResult, InterceptPayload};

pub const EVENT_PERMISSION_REQUESTED: &str = "permission_requested";
pub const EVENT_COMMAND_EXECUTED: &str = "command_executed";

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
}
