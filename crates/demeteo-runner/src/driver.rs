//! One push driver per `run_id`. A second caller never starts a second
//! driver and never returns early on its own: it records a re-arm request
//! that the live driver consumes in [`DriverGuard::finish_or_rearm`].
//!
//! The request and the release are decided under one lock. Checking the flag
//! and removing the slot in two separate critical sections would let a
//! request land in between and be dropped, leaving the run `running` with no
//! driver.
//!
//! The same holds one level up for a caller that re-opens a run: it takes the
//! slot before it writes the row, and an attacher writes nothing, so it cannot
//! overwrite a status the live driver settled while it was attaching.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Default)]
struct DriverSlot {
    rerun_requested: bool,
}

#[derive(Default)]
pub struct RunDrivers {
    slots: Mutex<HashMap<String, DriverSlot>>,
}

pub enum Acquire {
    Driver(DriverGuard),
    Attached,
}

#[must_use = "dropping the guard releases the run's driver slot"]
pub struct DriverGuard {
    drivers: Arc<RunDrivers>,
    run_id: String,
    released: bool,
}

impl RunDrivers {
    pub fn try_acquire(self: &Arc<Self>, run_id: &str) -> Acquire {
        let mut slots = self.lock();
        if let Some(slot) = slots.get_mut(run_id) {
            slot.rerun_requested = true;
            return Acquire::Attached;
        }
        slots.insert(run_id.to_string(), DriverSlot::default());
        Acquire::Driver(DriverGuard {
            drivers: Arc::clone(self),
            run_id: run_id.to_string(),
            released: false,
        })
    }

    fn lock(&self) -> MutexGuard<'_, HashMap<String, DriverSlot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl DriverGuard {
    /// `true`: a re-arm was requested while driving; the flag is cleared and
    /// the caller must run one more pass. `false`: the slot is released and
    /// the caller must stop.
    pub fn finish_or_rearm(&mut self) -> bool {
        let mut slots = self.drivers.lock();
        match slots.get_mut(&self.run_id) {
            Some(slot) if slot.rerun_requested => {
                slot.rerun_requested = false;
                true
            }
            _ => {
                slots.remove(&self.run_id);
                self.released = true;
                false
            }
        }
    }
}

impl Drop for DriverGuard {
    fn drop(&mut self) {
        if !self.released {
            self.drivers.lock().remove(&self.run_id);
        }
    }
}

#[cfg(test)]
#[path = "driver_tests.rs"]
mod tests;
