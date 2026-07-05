//! Tracks which features currently have a live execution driver.
//!
//! The driver loop is spawned with `tokio::spawn` and its `JoinHandle` is
//! dropped (we don't need to await it). The registry's job is just to
//! deduplicate spawns: if `ensure_running` is called for a feature that
//! already has a live driver, it's a no-op. If the driver dies (panic,
//! app restart, normal completion), the spawned wrapper deregisters it so
//! the next call to `ensure_running` starts a fresh one.
//!
//! Why not `JoinHandle::is_finished`? That requires `&mut`, which is
//! awkward to share across an `Arc<Mutex<HashMap<…>>>`. The
//! deregister-on-drop pattern is simpler and works for the cases we care
//! about: normal completion, panic propagation, and explicit cancellation.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::ids::FeatureId;

#[derive(Default)]
pub struct DriverRegistry {
    live: Arc<Mutex<HashMap<FeatureId, ()>>>,
}

impl DriverRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Returns `true` if a driver is currently registered for `feature_id`.
    pub fn is_live(&self, feature_id: &FeatureId) -> bool {
        self.live.lock().unwrap().contains_key(feature_id)
    }

    /// Register a driver. Call this *before* spawning so concurrent callers
    /// see the live entry and skip their own spawn.
    pub fn register(&self, feature_id: FeatureId) {
        self.live.lock().unwrap().insert(feature_id, ());
    }

    /// Remove a driver entry. Called by the spawned wrapper after the
    /// driver future completes (success, failure, panic, or cancellation).
    pub fn deregister(&self, feature_id: &FeatureId) {
        self.live.lock().unwrap().remove(feature_id);
    }
}
