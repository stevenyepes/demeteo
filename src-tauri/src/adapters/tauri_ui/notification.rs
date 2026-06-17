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

    fn emit_feature_status_changed(&self, feature_id: &str, status: &str) -> Result<(), String> {
        let body = serde_json::json!({
            "feature_id": feature_id,
            "status": status,
        });
        self.app
            .emit(crate::ports::notification::EVENT_FEATURE_STATUS_CHANGED, body)
            .map_err(|e| format!("Failed to emit feature_status_changed: {}", e))
    }

    fn emit_step_progress(
        &self,
        feature_id: &str,
        step_id: &str,
        status: &str,
        cost_usd: Option<f64>,
        wall_clock_secs: Option<u64>,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "feature_id": feature_id,
            "step_id": step_id,
            "status": status,
            "cost_usd": cost_usd,
            "wall_clock_secs": wall_clock_secs,
        });
        self.app
            .emit(crate::ports::notification::EVENT_STEP_PROGRESS, body)
            .map_err(|e| format!("Failed to emit step_progress: {}", e))
    }

    fn emit_gate_required(&self, feature_id: &str, step_execution_id: &str) -> Result<(), String> {
        let body = serde_json::json!({
            "feature_id": feature_id,
            "step_execution_id": step_execution_id,
        });
        self.app
            .emit(crate::ports::notification::EVENT_GATE_REQUIRED, body)
            .map_err(|e| format!("Failed to emit gate_required: {}", e))
    }

    fn emit_conflict_detected(&self, feature_id: &str, subtask_id: &str) -> Result<(), String> {
        let body = serde_json::json!({
            "feature_id": feature_id,
            "subtask_id": subtask_id,
        });
        self.app
            .emit(crate::ports::notification::EVENT_CONFLICT_DETECTED, body)
            .map_err(|e| format!("Failed to emit conflict_detected: {}", e))
    }
}
