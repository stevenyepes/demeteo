use crate::ports::notification::{DomainEvent, NotificationPort};
use tauri::{AppHandle, Emitter};

pub struct TauriNotificationAdapter {
    app: AppHandle,
}

impl TauriNotificationAdapter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl NotificationPort for TauriNotificationAdapter {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        // Map each `DomainEvent` variant to its (event_name, body) pair.
        // The body is the `serde_json::Value` form of the *same* event
        // (we strip the `kind` tag and emit just the inner data for
        // `PermissionRequested` to preserve the original wire format
        // where the payload was the bare `InterceptPayload`).
        let (name, body): (&str, serde_json::Value) = match event {
            DomainEvent::PermissionRequested(payload) => (
                "permission_requested",
                serde_json::to_value(payload).map_err(|e| e.to_string())?,
            ),
            DomainEvent::CommandExecuted { .. } => (
                "command_executed",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::FeatureStatusChanged { .. } => (
                "feature_status_changed",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::StepProgress { .. } => (
                "step_progress",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::GateRequired { .. } => (
                "gate_required",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::ConflictDetected { .. } => (
                "conflict_detected",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::AgentStream { .. } => (
                "agent_stream",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::MrMerged { .. } => (
                "mr_merged",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
        };

        self.app
            .emit(name, body)
            .map_err(|e| format!("Failed to emit {}: {}", name, e))
    }
}
