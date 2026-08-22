//! The three events one turn puts on the wire, and the shape each carries.
//!
//! Their own names rather than the `agent_event` / `thread_status_changed`
//! pair `crate::application::agents` emits: a Discovery is not a thread, and a
//! surface subscribed to both would have to tell them apart by payload.

use std::sync::{Arc, Mutex};

use serde::Serialize;

use crate::domain::agent_event::AgentEvent;
use crate::domain::models::Discovery;

/// Every `AgentEvent` of a turn, as it arrives.
pub const EVENT_DISCOVERY_AGENT_EVENT: &str = "discovery_agent_event";
/// A turn's phase, in the order they arrive: [`STATUS_SETTING_UP`],
/// [`STATUS_RUNNING`], then [`STATUS_IDLE`] or [`STATUS_ERROR`].
pub const EVENT_DISCOVERY_TURN_STATUS: &str = "discovery_turn_status";

/// The turn has been claimed and is being resolved: which repository on which
/// host, a worktree if this Discovery no longer has one (§4.6), the
/// attachments, the transcript, and what the harness reports about itself.
///
/// A phase of its own rather than an early [`STATUS_RUNNING`], because the
/// surface renders that one as the agent thinking — which through a
/// re-provision is a claim about a process that has not been started yet. What
/// it may honestly say here is in `docs/DISCOVERY_UI_SPEC.md` §3.4.3.
pub const STATUS_SETTING_UP: &str = "setting_up";
/// The agent has the turn.
pub const STATUS_RUNNING: &str = "running";
/// Nothing is running. The one a surface refreshes on, which is why every
/// claim is released before it is sent.
pub const STATUS_IDLE: &str = "idle";
/// Nothing is running, and `reason` says what stopped it.
pub const STATUS_ERROR: &str = "error";
/// The completion signal §4.3 asks for. Leaving mid-interview is the case the
/// feature exists for, so a multi-minute turn that ends silently forces the
/// user to sit and watch.
pub const EVENT_DISCOVERY_TURN_COMPLETED: &str = "discovery_turn_completed";

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryTurnStatus {
    pub discovery_id: String,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscoveryTurnCompleted {
    pub discovery_id: String,
    /// The Discovery's title, so a toast can name what finished without a
    /// round trip to fetch it.
    pub title: String,
    /// `None` when the turn said nothing worth keeping, which is the shape of
    /// every failed one.
    pub message_id: Option<String>,
    pub ending: &'static str,
    pub reason: Option<String>,
    pub cost_usd: f64,
    pub tokens: i64,
    pub duration_ms: u64,
    /// Whether the turn had to carry the transcript itself — the difference
    /// §3.4.3 of `docs/DISCOVERY_UI_SPEC.md` renders under the bubble.
    pub reseeded: bool,
    pub nothing_left_to_settle: bool,
}

/// How a turn stopped, collapsed from the harness's own vocabulary to the
/// four endings a surface can do anything about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnEnding {
    Success,
    Interrupted,
    Failed,
    Environmental,
}

impl TurnEnding {
    /// The stable identifier the completion event carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Interrupted => "interrupted",
            Self::Failed => "failed",
            Self::Environmental => "environmental",
        }
    }
}

pub(crate) fn status_payload(
    discovery: &Discovery,
    status: &str,
    reason: Option<String>,
) -> serde_json::Value {
    serde_json::to_value(DiscoveryTurnStatus {
        discovery_id: discovery.id.as_str().to_string(),
        status: status.to_string(),
        reason,
    })
    .unwrap_or(serde_json::Value::Null)
}

/// Coalesces text deltas so a fast-streaming turn does not wake the webview
/// once per token. Non-text events flush first, so nothing arrives out of the
/// order the agent produced it in.
pub(crate) struct Sink<F> {
    emit: Arc<F>,
    discovery_id: String,
    pending: Mutex<(String, std::time::Instant)>,
}

impl<F> Sink<F>
where
    F: Fn(&str, serde_json::Value) + Send + Sync + 'static,
{
    const COALESCE_MS: u128 = 50;

    pub(crate) fn new(emit: Arc<F>, discovery_id: String) -> Self {
        Self {
            emit,
            discovery_id,
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
            EVENT_DISCOVERY_AGENT_EVENT,
            serde_json::json!({ "discovery_id": self.discovery_id, "event": event }),
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/application/discovery/events.rs"]
mod tests;
