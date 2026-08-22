//! Which Discoveries have a turn running *in this process*, and the only
//! thing that can answer it.
//!
//! The reason nothing durable holds this is
//! [`SyncTurns`](crate::application::sync_turns::SyncTurns)'s, stated there
//! and not repeated: a row claiming a live turn outlives the process that
//! wrote it. What is different here is only who asks. A turn is spawned and
//! reports itself through events, so the surface that started it knows;
//! `discovery_get` is for the surface that did *not* — one opened after
//! navigating away mid-turn, which has heard nothing and would otherwise draw
//! an idle interview over a running one.
//!
//! Counted rather than exclusive. Nothing here refuses a second turn — that is
//! the composer's job, and this is a read model — so two overlapping turns
//! must not have the first to finish report the second as over.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::ids::DiscoveryId;

#[derive(Default)]
pub struct RunningTurns {
    counts: Mutex<HashMap<String, usize>>,
}

impl RunningTurns {
    /// Mark a turn as running until the returned guard drops.
    ///
    /// Taken by `Arc` rather than by reference because a claim outlives the
    /// scope that took it: an interview turn is claimed on the task that
    /// prepares it and released on the spawned one that runs it, so a guard
    /// borrowing this would be a guard that could not cross the spawn.
    pub fn claim(self: Arc<Self>, discovery_id: &str) -> RunningTurn {
        *self.lock().entry(discovery_id.to_string()).or_insert(0) += 1;
        RunningTurn {
            discovery_id: discovery_id.to_string(),
            turns: self,
        }
    }

    pub fn running(&self, discovery_id: &DiscoveryId) -> bool {
        self.lock().contains_key(discovery_id.as_str())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, usize>> {
        self.counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One turn's claim, held for as long as this lives — including through the
/// `?` of a pass that gave up early and the drop of a future nobody polls any
/// more, which is what stops a Discovery reporting a turn that ended with the
/// task that was running it.
pub struct RunningTurn {
    turns: Arc<RunningTurns>,
    discovery_id: String,
}

impl Drop for RunningTurn {
    fn drop(&mut self) {
        let mut counts = self.turns.lock();
        match counts.get_mut(&self.discovery_id) {
            Some(count) if *count > 1 => *count -= 1,
            _ => {
                counts.remove(&self.discovery_id);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/application/discovery/running.rs"]
mod tests;
