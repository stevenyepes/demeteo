//! In-memory rendezvous between a parked `/authorize` handler and the Tauri
//! `mcp_consent_decide` command that resolves it, keyed by `request_id`.
//!
//! Same shape as [`crate::adapters::step_executor::gate_waiter::GateWaiter`]
//! (`Notify` + a shared decision slot) but deliberately simpler: there is no
//! DB reconciliation, because a `PendingAuthorization` lost to an app
//! restart is cheap to retry — the MCP client just re-issues `/authorize` —
//! unlike a `Gate` mid-run, which a driver may already be blocked on.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    Approved,
    Denied,
}

#[derive(Default)]
pub struct McpConsentWaiterRegistry {
    waiters: Mutex<Waiters>,
}

#[derive(Default)]
struct Waiters {
    notifiers: HashMap<String, Arc<Notify>>,
    decisions: HashMap<String, ConsentDecision>,
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
        self.waiters
            .lock()
            .notifiers
            .insert(request_id.to_string(), Arc::clone(&notify));
        notify
    }

    /// Register `request_id` only while fewer than `max_pending` requests are
    /// parked, checked and inserted under one lock. The returned
    /// [`ParkedConsent`] removes the registration when dropped — including when
    /// axum drops the handler future because the client disconnected, which
    /// would otherwise leave the slot held until the process exits.
    pub fn try_park(&self, request_id: &str, max_pending: usize) -> Option<ParkedConsent<'_>> {
        let notify = Arc::new(Notify::new());
        {
            let mut waiters = self.waiters.lock();
            if waiters.notifiers.len() >= max_pending {
                return None;
            }
            waiters
                .notifiers
                .insert(request_id.to_string(), Arc::clone(&notify));
        }
        Some(ParkedConsent {
            registry: self,
            request_id: request_id.to_string(),
            notify,
        })
    }

    /// Record `decision` for `request_id` and wake its waiter. A second
    /// delivery for an id that already has a decision is a no-op — only the
    /// first consent decision is observed.
    ///
    /// Returns `false` when no such request is pending — it timed out, was
    /// abandoned by its client, or never existed — so the caller can tell the
    /// human their answer went nowhere instead of letting them believe it was
    /// honoured.
    pub fn deliver(&self, request_id: &str, decision: ConsentDecision) -> bool {
        let notify = {
            let mut waiters = self.waiters.lock();
            let Some(notify) = waiters.notifiers.get(request_id).cloned() else {
                return false;
            };
            if waiters.decisions.contains_key(request_id) {
                return true;
            }
            waiters.decisions.insert(request_id.to_string(), decision);
            notify
        };
        notify.notify_one();
        true
    }

    /// Consume and return the decision for `request_id`, if any, removing
    /// both it and its notifier so a resolved request doesn't linger in
    /// either map for the rest of the app's lifetime.
    pub fn take_decision(&self, request_id: &str) -> Option<ConsentDecision> {
        let mut waiters = self.waiters.lock();
        waiters.notifiers.remove(request_id);
        waiters.decisions.remove(request_id)
    }
}

/// A request parked by [`McpConsentWaiterRegistry::try_park`].
pub struct ParkedConsent<'a> {
    registry: &'a McpConsentWaiterRegistry,
    request_id: String,
    notify: Arc<Notify>,
}

impl ParkedConsent<'_> {
    /// Wait up to `timeout` for a decision. `None` means none arrived in time.
    pub async fn decision(&self, timeout: Duration) -> Option<ConsentDecision> {
        let _ = tokio::time::timeout(timeout, self.notify.notified()).await;
        self.registry.take_decision(&self.request_id)
    }
}

impl Drop for ParkedConsent<'_> {
    fn drop(&mut self) {
        self.registry.take_decision(&self.request_id);
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
    fn deliver_reports_whether_anything_was_waiting() {
        let registry = McpConsentWaiterRegistry::new();
        assert!(!registry.deliver("nobody", ConsentDecision::Approved));

        registry.register("req-5");
        assert!(registry.deliver("req-5", ConsentDecision::Approved));
        registry.take_decision("req-5");
        assert!(
            !registry.deliver("req-5", ConsentDecision::Approved),
            "a request that has been taken is no longer pending"
        );
    }

    #[test]
    fn try_park_refuses_past_the_cap_and_a_dropped_park_frees_its_slot() {
        let registry = McpConsentWaiterRegistry::new();
        let first = registry.try_park("a", 1).expect("room for one");
        assert!(registry.try_park("b", 1).is_none(), "the cap is 1");

        drop(first);

        assert!(registry.try_park("b", 1).is_some());
        assert!(
            !registry.deliver("a", ConsentDecision::Approved),
            "an abandoned request cannot be approved"
        );
    }

    #[test]
    fn take_decision_returns_none_for_unknown_id() {
        let registry = McpConsentWaiterRegistry::new();
        assert_eq!(registry.take_decision("missing"), None);
    }

    #[test]
    fn late_delivery_for_a_resolved_request_is_discarded() {
        let registry = McpConsentWaiterRegistry::new();
        registry.register("req-4");
        assert_eq!(registry.take_decision("req-4"), None);

        registry.deliver("req-4", ConsentDecision::Approved);

        let waiters = registry.waiters.lock();
        assert!(!waiters.decisions.contains_key("req-4"));
        assert!(!waiters.notifiers.contains_key("req-4"));
    }
}
