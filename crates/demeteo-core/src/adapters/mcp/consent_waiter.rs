//! In-memory rendezvous between a parked `/authorize` handler and the Tauri
//! `mcp_consent_decide` command that resolves it, keyed by `request_id`.
//!
//! Same shape as [`crate::adapters::step_executor::gate_waiter::GateWaiter`]
//! (`Notify` + a shared decision slot) but deliberately simpler: there is no
//! DB reconciliation, because a `PendingAuthorization` lost to an app
//! restart is cheap to retry — the MCP client just re-issues `/authorize` —
//! unlike a `Gate` mid-run, which a driver may already be blocked on.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    Approved,
    Denied,
}

#[derive(Default)]
pub struct McpConsentWaiterRegistry {
    notifiers: Mutex<HashMap<String, Arc<Notify>>>,
    decisions: Mutex<HashMap<String, ConsentDecision>>,
}

impl McpConsentWaiterRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `request_id` and return the `Notify` the parked handler
    /// should `.notified().await` on (with its own timeout — this registry
    /// has no opinion on how long to wait).
    pub fn register(&self, request_id: &str) -> Arc<Notify> {
        let notify = Arc::new(Notify::new());
        self.notifiers
            .lock()
            .unwrap()
            .insert(request_id.to_string(), Arc::clone(&notify));
        notify
    }

    /// Record `decision` for `request_id` and wake its waiter. A second
    /// delivery for an id that already has a decision is a no-op — only the
    /// first consent decision is observed.
    pub fn deliver(&self, request_id: &str, decision: ConsentDecision) {
        {
            let mut decisions = self.decisions.lock().unwrap();
            if decisions.contains_key(request_id) {
                return;
            }
            decisions.insert(request_id.to_string(), decision);
        }
        if let Some(notify) = self.notifiers.lock().unwrap().get(request_id) {
            notify.notify_one();
        }
    }

    /// Consume and return the decision for `request_id`, if any, removing
    /// both it and its notifier so a resolved request doesn't linger in
    /// either map for the rest of the app's lifetime.
    pub fn take_decision(&self, request_id: &str) -> Option<ConsentDecision> {
        let decision = self.decisions.lock().unwrap().remove(request_id);
        self.notifiers.lock().unwrap().remove(request_id);
        decision
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn delivering_a_decision_wakes_a_parked_waiter() {
        let registry = Arc::new(McpConsentWaiterRegistry::new());
        let notify = registry.register("req-1");

        let waiter = tokio::spawn(async move {
            tokio::time::timeout(Duration::from_secs(1), notify.notified()).await
        });
        tokio::task::yield_now().await;

        registry.deliver("req-1", ConsentDecision::Approved);

        waiter
            .await
            .expect("waiter task panicked")
            .expect("waiter should wake once delivered");
        assert_eq!(
            registry.take_decision("req-1"),
            Some(ConsentDecision::Approved)
        );
    }

    #[test]
    fn second_delivery_for_the_same_id_is_a_no_op() {
        let registry = McpConsentWaiterRegistry::new();
        registry.register("req-2");

        registry.deliver("req-2", ConsentDecision::Approved);
        registry.deliver("req-2", ConsentDecision::Denied);

        assert_eq!(
            registry.take_decision("req-2"),
            Some(ConsentDecision::Approved)
        );
    }

    #[test]
    fn take_decision_consumes_the_decision_once() {
        let registry = McpConsentWaiterRegistry::new();
        registry.register("req-3");
        registry.deliver("req-3", ConsentDecision::Denied);

        assert_eq!(
            registry.take_decision("req-3"),
            Some(ConsentDecision::Denied)
        );
        assert_eq!(registry.take_decision("req-3"), None);
    }

    #[test]
    fn take_decision_returns_none_for_unknown_id() {
        let registry = McpConsentWaiterRegistry::new();
        assert_eq!(registry.take_decision("missing"), None);
    }
}
