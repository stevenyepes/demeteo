use crate::domain::intercept::{ExecutionResult, InterceptPayload};
use crate::ports::notification::{NotificationPort, EVENT_COMMAND_EXECUTED, EVENT_PERMISSION_REQUESTED};
use tauri::AppHandle;
use tauri::Emitter;

pub struct TauriNotificationAdapter {
    app: AppHandle,
}

impl TauriNotificationAdapter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl NotificationPort for TauriNotificationAdapter {
    fn emit_permission_requested(&self, payload: &InterceptPayload) -> Result<(), String> {
        self.app
            .emit(EVENT_PERMISSION_REQUESTED, payload.clone())
            .map_err(|e| format!("Failed to emit permission_requested: {}", e))
    }

    fn emit_command_executed(
        &self,
        thread_id: &str,
        machine_id: &str,
        result: &ExecutionResult,
        intercept_id: Option<&str>,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "thread_id": thread_id,
            "machine_id": machine_id,
            "result": result,
            "intercept_id": intercept_id,
        });
        self.app
            .emit(EVENT_COMMAND_EXECUTED, body)
            .map_err(|e| format!("Failed to emit command_executed: {}", e))
    }
}
