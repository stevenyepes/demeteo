//! The three events one Ask turn puts on the wire, and the shape each carries.
//!
//! Named and valued distinctly from `discovery::events`'s trio — a surface
//! subscribed to both must be able to tell them apart by event name alone,
//! and Ask is not a Discovery (no decompose, no reseed-visible UI contract).

use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::domain::agent_event::AgentEvent;
use crate::domain::models::AskThread;

/// Every `AgentEvent` of a turn, as it arrives.
pub const EVENT_ASK_AGENT_EVENT: &str = "ask_agent_event";
/// A turn's phase, in the order they arrive: [`STATUS_SETTING_UP`],
/// [`STATUS_RUNNING`], then [`STATUS_IDLE`] or [`STATUS_ERROR`].
pub const EVENT_ASK_TURN_STATUS: &str = "ask_turn_status";
/// The completion signal a subscribed surface waits on to know a turn ended.
pub const EVENT_ASK_TURN_COMPLETED: &str = "ask_turn_completed";

/// The turn has been claimed and is being resolved: which worktree, which
/// host, and what the harness reports about itself. Ask's own const, not
/// shared with `discovery::events::STATUS_SETTING_UP` — see AGENTS.md §3 on
/// not building a shared-status abstraction ahead of a second need for one.
pub const STATUS_SETTING_UP: &str = "setting_up";
/// The agent has the turn.
pub const STATUS_RUNNING: &str = "running";
/// Nothing is running. The one a surface refreshes on, which is why every
/// claim is released before it is sent.
pub const STATUS_IDLE: &str = "idle";
/// Nothing is running, and `reason` says what stopped it.
pub const STATUS_ERROR: &str = "error";

#[derive(Debug, Clone, Serialize)]
pub struct AskTurnStatus {
    pub thread_id: String,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AskTurnCompleted {
    pub thread_id: String,
    /// The thread's title, so a toast can name what finished without a round
    /// trip to fetch it.
    pub title: String,
    /// `None` when the turn said nothing worth keeping, which is the shape of
    /// every failed one.
    pub message_id: Option<String>,
    pub ending: &'static str,
    pub reason: Option<String>,
    pub cost_usd: f64,
    pub tokens: i64,
    pub duration_ms: u64,
}

pub(crate) fn status_payload(
    thread: &AskThread,
    status: &str,
    reason: Option<String>,
) -> serde_json::Value {
    serde_json::to_value(AskTurnStatus {
        thread_id: thread.id.as_str().to_string(),
        status: status.to_string(),
        reason,
    })
    .unwrap_or(serde_json::Value::Null)
}

/// The same coalescing mechanism as
/// [`discovery::events::Sink`](crate::application::discovery::events::Sink),
/// retargeted to Ask's event/status types.
pub(crate) struct Sink<F> {
    emit: Arc<F>,
    thread_id: String,
    pending: Mutex<(String, std::time::Instant)>,
}

impl<F> Sink<F>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    const COALESCE_MS: u128 = 50;

    pub(crate) fn new(emit: Arc<F>, thread_id: String) -> Self {
        Self {
            emit,
            thread_id,
            pending: Mutex::new((String::new(), std::time::Instant::now())),
        }
    }

    pub(crate) fn push(&self, event: &AgentEvent) {
        if let AgentEvent::Text { delta } = event {
            let mut pending = match self.pending.lock() {
                Ok(p) => p,
                Err(_) => return,
            };
            pending.0.push_str(delta);
            if pending.1.elapsed().as_millis() < Self::COALESCE_MS {
                return;
            }
            let batched = std::mem::take(&mut pending.0);
            pending.1 = std::time::Instant::now();
            drop(pending);
            self.send(&AgentEvent::Text { delta: batched });
            return;
        }
        self.flush();
        self.send(event);
    }

    pub(crate) fn flush(&self) {
        let batched = match self.pending.lock() {
            Ok(mut pending) => std::mem::take(&mut pending.0),
            Err(_) => return,
        };
        if !batched.is_empty() {
            self.send(&AgentEvent::Text { delta: batched });
        }
    }

    fn send(&self, event: &AgentEvent) {
        (self.emit)(
            EVENT_ASK_AGENT_EVENT,
            serde_json::json!({ "thread_id": self.thread_id, "event": event }),
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/application/ask/events.rs"]
mod tests;
