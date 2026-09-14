//! Feature-level status reconciliation (`ExecutionDriver`).
//!
//! The decisions live in `updates.rs` as free functions over the two ports
//! they read, so a test reaches them without a driver; these are the
//! driver-flavoured spellings the run loop and the gate step call.

use super::ExecutionDriver;
use crate::adapters::step_executor::updates;

impl ExecutionDriver {
    /// See [`updates::ensure_feature_running`].
    pub(crate) fn ensure_feature_running(&self) {
        updates::ensure_feature_running(&*self.features, &*self.notif, &self.f_id);
    }

    /// See [`updates::park_feature`].
    pub(crate) fn park_feature(&self) {
        updates::park_feature(&*self.features, &*self.notif, &self.f_id);
    }
}
