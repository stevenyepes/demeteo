//! Durable rendezvous between a gate-deciding call (`gate_decide`) and the
//! driver loop that is currently waiting on that gate.
//!
//! The DB row in `gate_decisions` is the source of truth; `GateWaiter` is
//! just a fast-path wakeup so the driver doesn't have to poll the DB while
//! waiting. The two halves cooperate via [`tokio::sync::Notify`] + a shared
//! `Option<GateDecision>` slot.
//!
//! Rationale: the previous design used `oneshot::Sender`, which has two
//! failure modes — the receiver can be dropped (driver died, app restart,
//! event emitted before listener registered), and the channel itself holds
//! the payload so a late sender has nothing to deliver to. Decoupling
//! "wakeup" from "payload" lets the DB reconcile any decision the in-memory
//! waiters never saw.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::Notify;

use crate::domain::ids::FeatureId;
use crate::domain::models::GateDecision;
use crate::domain::step_seed::belongs_to_feature;

/// Drop `feature_id`'s waiters from the process-wide registry, and no one
/// else's — the map is keyed by step-execution id and shared by every live
/// driver, so a teardown holds its peers' keys as well as its own.
///
/// A free function over the map alone because the only caller is a driver
/// teardown, and reaching that in a test means constructing an
/// `ExecutionDriver` — twenty-odd ports, none of which this reads. The
/// scoping is the part that was wrong; this is where it can be asserted.
pub fn sweep_feature(waiters: &Mutex<HashMap<String, Arc<GateWaiter>>>, feature_id: &FeatureId) {
    let Ok(mut map) = waiters.lock() else {
        return;
    };
    map.retain(|key, _| !belongs_to_feature(feature_id, key));
}

#[derive(Default)]
pub struct GateWaiter {
    notify: Notify,
    decision: Mutex<Option<GateDecision>>,
}

impl GateWaiter {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Deliver a decision and wake one waiter. Safe to call multiple times;
    /// only the first delivery is observed by the driver (subsequent
    /// deliveries are persisted to the DB by the caller and picked up on
    /// driver restart).
    pub fn deliver(self: &Arc<Self>, decision: GateDecision) {
        if let Ok(mut slot) = self.decision.lock() {
            if slot.is_some() {
                // Already delivered; idempotent no-op.
                return;
            }
            *slot = Some(decision);
        }
        self.notify.notify_one();
    }

    /// Wait for a decision. Returns `None` if the future is cancelled
    /// before any decision arrives.
    pub async fn wait(self: &Arc<Self>) -> Option<GateDecision> {
        loop {
            // Check before notifying so we don't miss a delivery that
            // happened between waiter creation and first poll.
            if let Ok(mut slot) = self.decision.lock() {
                if let Some(d) = slot.take() {
                    return Some(d);
                }
            }
            self.notify.notified().await;
        }
    }
}

use std::sync::Arc;

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/gate_waiter.rs"]
mod tests;
