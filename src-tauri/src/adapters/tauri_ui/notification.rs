use crate::ports::notification::{DomainEvent, NotificationPort};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::{NotificationExt, PermissionState};

/// The label of the `"main"` webview window whose visibility/focus gates
/// OS notifications.
const MAIN_WINDOW: &str = "main";

pub struct TauriNotificationAdapter {
    app: AppHandle,
}

impl TauriNotificationAdapter {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

/// Decide whether a domain event should also surface as a native OS
/// notification, and with what `(title, body)` text.
///
/// This is the pure routing decision (spec §5.1) — it returns `Some(_)` **only**
/// for the user-facing terminal events (a feature reaching `completed`/`failed`,
/// a gate awaiting input, an MR merge, an exhausted retry budget, an
/// environment problem, or a merge conflict) and `None` for every progress /
/// telemetry / permission event that already has an in-app surface.
///
/// The window-visibility/focus gate is applied by the caller
/// ([`TauriNotificationAdapter::emit`]), not here, so this stays testable
/// without a live window.
fn os_notification_for(event: &DomainEvent) -> Option<(String, String)> {
    match event {
        // A feature only warrants an OS notification when it *finishes* —
        // `running`/`paused`/etc. are progress noise the in-app UI already shows.
        DomainEvent::FeatureStatusChanged { feature_id, status } => match status.as_str() {
            "completed" => Some((
                "Feature completed".to_string(),
                format!("{feature_id} finished successfully."),
            )),
            "failed" => Some((
                "Feature failed".to_string(),
                format!("{feature_id} failed."),
            )),
            _ => None,
        },
        DomainEvent::GateRequired { feature_id, .. } => Some((
            "Approval required".to_string(),
            format!("{feature_id} is waiting for your input at a gate."),
        )),
        DomainEvent::MrMerged { feature_title, .. } => Some((
            "Merge request merged".to_string(),
            format!("\"{feature_title}\" was merged."),
        )),
        DomainEvent::RetryBudgetExhausted {
            feature_id, step_id, ..
        } => Some((
            "Retry budget exhausted".to_string(),
            format!("{feature_id} gave up on step {step_id} after exhausting its retries."),
        )),
        DomainEvent::EnvironmentNotReady {
            feature_id, reason, ..
        } => Some((
            "Environment not ready".to_string(),
            format!("{feature_id}: {reason}"),
        )),
        DomainEvent::ConflictDetected {
            feature_id,
            subtask_id,
        } => Some((
            "Merge conflict detected".to_string(),
            format!("{feature_id} has a conflict on subtask {subtask_id}."),
        )),
        // Progress / telemetry / permission events never fire an OS notification:
        // AgentStream, StepProgress, BootstrapProgress, CommandExecuted,
        // AgentSpawned, PermissionRequested (spec §5.1 / AC-6).
        _ => None,
    }
}

/// Best-effort check that the OS will actually deliver a notification before
/// we build one.
///
/// On macOS (and some Linux backends) `tauri-plugin-notification` gates delivery
/// behind a runtime permission grant; calling `.show()` without it silently
/// no-ops. We consult the current permission state and, when it has never been
/// decided (`Prompt`/`PromptWithRationale`), request it once:
///   * `Granted`  → proceed with the show.
///   * `Denied`   → skip the pointless show (the user opted out).
///   * `Prompt*`  → request once; only show if the grant comes back `Granted`.
///   * query error → fall through to a best-effort show, so a permissionless
///     backend (typical Linux desktop, where the plugin reports `Granted`
///     anyway) still notifies rather than being silently suppressed.
///
/// Kept a free function taking `&AppHandle` so the caller stays readable; the
/// desktop plugin reports `Granted` unconditionally, so this is a no-op there
/// and the real gate only matters on macOS / mobile.
fn os_notifications_permitted(app: &AppHandle) -> bool {
    let notification = app.notification();
    match notification.permission_state() {
        Ok(PermissionState::Granted) => true,
        Ok(PermissionState::Denied) => false,
        Ok(_) => matches!(
            notification.request_permission(),
            Ok(PermissionState::Granted)
        ),
        Err(_) => true,
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
            DomainEvent::AgentSpawned { .. } => (
                "agent_spawned",
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
            DomainEvent::RetryBudgetExhausted { .. } => (
                "retry_budget_exhausted",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::EnvironmentNotReady { .. } => (
                "environment_not_ready",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
            DomainEvent::BootstrapProgress { .. } => (
                "bootstrap_progress",
                serde_json::to_value(event).map_err(|e| e.to_string())?,
            ),
        };

        // In-app emit is preserved exactly (spec Constraint 2 / AC-7).
        let emit_result = self
            .app
            .emit(name, body)
            .map_err(|e| format!("Failed to emit {}: {}", name, e));

        // Additively route user-facing terminal events to a native OS
        // notification, but only while the main window is hidden or unfocused —
        // when it is visible and focused the in-app toast/bell already covers it
        // (spec AC-6 / Constraint 3). A missing window is treated as hidden.
        if let Some((title, os_body)) = os_notification_for(event) {
            let window_hidden_or_unfocused = match self.app.get_webview_window(MAIN_WINDOW) {
                Some(window) => {
                    let visible = window.is_visible().unwrap_or(false);
                    let focused = window.is_focused().unwrap_or(false);
                    !visible || !focused
                }
                None => true,
            };
            // Only build the notification when the window is out of view *and*
            // the OS will actually deliver it — an ungranted permission would
            // otherwise make the show silently no-op (macOS/mobile), leaving
            // AC-6 unobservably false.
            if window_hidden_or_unfocused && os_notifications_permitted(&self.app) {
                // A failed OS notification must never break the in-app emit path,
                // so the result is intentionally ignored (best-effort surface).
                let _ = self
                    .app
                    .notification()
                    .builder()
                    .title(title)
                    .body(os_body)
                    .show();
            }
        }

        emit_result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::action::ActionKind;
    use crate::domain::ids::{
        FeatureId, InterceptId, MachineId, StepExecutionId, ThreadId,
    };
    use crate::domain::intercept::{ExecutionResult, InterceptPayload};

    fn feature() -> FeatureId {
        FeatureId::new("feat-1")
    }

    /// A `completed`/`failed` feature is the canonical terminal event. Beyond
    /// the Some/None routing decision, pin the produced text: both parts must be
    /// non-empty and the body must name the feature, so an accidental empty
    /// string or a swapped `title`/`body` never ships a blank OS notification.
    #[test]
    fn feature_completed_and_failed_notify() {
        let completed = DomainEvent::FeatureStatusChanged {
            feature_id: feature(),
            status: "completed".to_string(),
        };
        let failed = DomainEvent::FeatureStatusChanged {
            feature_id: feature(),
            status: "failed".to_string(),
        };
        for event in [&completed, &failed] {
            let (title, body) = os_notification_for(event)
                .unwrap_or_else(|| panic!("expected Some for {event:?}"));
            assert!(!title.is_empty(), "title must not be empty for {event:?}");
            assert!(!body.is_empty(), "body must not be empty for {event:?}");
            assert!(
                body.contains("feat-1"),
                "body must name the feature for {event:?}, got {body:?}"
            );
        }
    }

    /// Any non-terminal feature status is progress noise — no OS notification
    /// (spec §5.1: `FeatureStatusChanged` with *any other status* → `None`).
    /// Cover a spread of realistic lifecycle values, not just `running`.
    #[test]
    fn feature_non_terminal_statuses_do_not_notify() {
        for status in ["running", "queued", "paused", "cancelled", "", "COMPLETED"] {
            let event = DomainEvent::FeatureStatusChanged {
                feature_id: feature(),
                status: status.to_string(),
            };
            assert!(
                os_notification_for(&event).is_none(),
                "expected None for status {status:?}"
            );
        }
    }

    /// The remaining user-facing terminal events all produce a notification.
    #[test]
    fn user_facing_terminal_events_notify() {
        let gate = DomainEvent::GateRequired {
            feature_id: feature(),
            step_execution_id: StepExecutionId::new("se-1"),
        };
        let mr = DomainEvent::MrMerged {
            feature_id: feature(),
            project_id: "p-1".to_string(),
            feature_title: "Add billing".to_string(),
            mr_url: "https://example/mr/1".to_string(),
        };
        let retry = DomainEvent::RetryBudgetExhausted {
            feature_id: feature(),
            step_id: "s-implement".to_string(),
            target_id: "s-implement".to_string(),
            attempt: 3,
            max: 3,
            reason: "gave up".to_string(),
        };
        let env = DomainEvent::EnvironmentNotReady {
            feature_id: feature(),
            step_id: "s-implement".to_string(),
            reason: "install rustc".to_string(),
        };
        let conflict = DomainEvent::ConflictDetected {
            feature_id: feature(),
            subtask_id: "st-1".to_string(),
        };
        for event in [gate, mr, retry, env, conflict] {
            let (title, body) = os_notification_for(&event)
                .unwrap_or_else(|| panic!("expected Some for {event:?}"));
            assert!(!title.is_empty(), "title must not be empty for {event:?}");
            assert!(!body.is_empty(), "body must not be empty for {event:?}");
        }
    }

    /// Every progress / telemetry / permission event stays silent (spec §5.1).
    #[test]
    fn progress_and_permission_events_do_not_notify() {
        let se = || StepExecutionId::new("se-1");
        let permission = DomainEvent::PermissionRequested(InterceptPayload {
            intercept_id: InterceptId::new("i-1"),
            thread_id: ThreadId::new("t-1"),
            machine_id: MachineId::new("m-1"),
            action: ActionKind::RunBash,
            target: "ls".to_string(),
            preview: None,
            created_at: "0".to_string(),
            tool_call_id: None,
        });
        let command_executed = DomainEvent::CommandExecuted {
            thread_id: "t-1".to_string(),
            machine_id: "m-1".to_string(),
            result: ExecutionResult::Bash {
                output: "ok".to_string(),
            },
            intercept_id: None,
        };
        let step_progress = DomainEvent::StepProgress {
            feature_id: feature(),
            step_id: "s-implement".to_string(),
            status: "running".to_string(),
            cost_usd: None,
            tokens: None,
            wall_clock_secs: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
        };
        let bootstrap = DomainEvent::BootstrapProgress {
            feature_id: feature(),
            phase: "connecting".to_string(),
            label: "Connecting".to_string(),
            status: "running".to_string(),
            detail: None,
        };
        let agent_spawned = DomainEvent::AgentSpawned {
            feature_id: feature(),
            step_execution_id: se(),
            agent_kind: "opencode".to_string(),
            model: None,
            effort: None,
        };
        let agent_stream = DomainEvent::AgentStream {
            feature_id: feature(),
            step_execution_id: se(),
            content: "hello".to_string(),
        };
        for event in [
            permission,
            command_executed,
            step_progress,
            bootstrap,
            agent_spawned,
            agent_stream,
        ] {
            assert!(
                os_notification_for(&event).is_none(),
                "expected None for {event:?}"
            );
        }
    }
}
