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

    /// Return an RAII guard that deregisters `feature_id` when dropped —
    /// including on panic unwind. The caller must already have called
    /// [`register`]; the guard owns only the *deregister* half.
    ///
    /// This exists because a trailing `deregister` statement after the
    /// driver future is skipped when that future panics, leaking the `live`
    /// entry so [`is_live`](Self::is_live) stays `true` forever. In-process
    /// recovery (`gate_decide` / retry, both via `ensure_driver_running`)
    /// then no-ops on the dead feature, and only a full app restart — which
    /// drops the in-memory registry — clears it.
    pub fn deregister_guard(self: Arc<Self>, feature_id: FeatureId) -> DriverGuard {
        DriverGuard {
            registry: self,
            feature_id,
        }
    }
}

/// Deregisters its feature's driver entry on drop; see
/// [`DriverRegistry::deregister_guard`].
pub struct DriverGuard {
    registry: Arc<DriverRegistry>,
    feature_id: FeatureId,
}

impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.registry.deregister(&self.feature_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{catch_unwind, AssertUnwindSafe};

    #[test]
    fn guard_deregisters_on_normal_drop() {
        let reg = DriverRegistry::new();
        let f = FeatureId::new("f-1");
        reg.register(f.clone());
        assert!(reg.is_live(&f));
        {
            let _guard = reg.clone().deregister_guard(f.clone());
            assert!(reg.is_live(&f));
        }
        assert!(!reg.is_live(&f), "guard should deregister when dropped");
    }

    #[test]
    fn guard_deregisters_on_panic_unwind() {
        // The whole point: a panicking driver task must not leak its entry.
        let reg = DriverRegistry::new();
        let f = FeatureId::new("f-panic");
        reg.register(f.clone());
        let reg2 = reg.clone();
        let f2 = f.clone();
        let result = catch_unwind(AssertUnwindSafe(|| {
            let _guard = reg2.clone().deregister_guard(f2.clone());
            panic!("driver blew up");
        }));
        assert!(result.is_err(), "closure should have panicked");
        assert!(
            !reg.is_live(&f),
            "guard's Drop must fire on unwind so is_live can't stay true forever"
        );
    }
}
