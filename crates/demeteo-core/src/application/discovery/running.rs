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
//! Exclusive to acquire, counted to release, and the two are the same
//! mechanism: [`RunningTurns::try_claim`] takes the counted claim and hands it
//! straight back when it was not the only one, so the refusal costs the
//! running turn nothing. Nothing outside this module may take a claim any
//! other way — a second turn on one Discovery does not queue behind the first,
//! it ends it, because `AgentSession::prompt`
//! (`crates/demeteo-core/src/adapters/agent/cli_runtime.rs`) kills the child
//! the previous turn is streaming from.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::ids::DiscoveryId;

/// What a caller is told when the Discovery already has a turn.
///
/// One wording for both callers because the claim does not record which kind
/// took it: an interview turn and a decompose pass are the same one-shot
/// invocation against the same session, and it names what is happening rather
/// than reporting that something failed.
pub const ALREADY_RUNNING: &str = "This discovery is already working — an interview turn or a \
                                   decompose pass is under way. Wait for it to finish before \
                                   starting another.";

#[derive(Default)]
pub struct RunningTurns {
    counts: Mutex<HashMap<String, usize>>,
}

impl RunningTurns {
    /// Take the turn only if this Discovery has none, which is the only way in.
    ///
    /// `None` is a refusal, not an error: the caller says so in its own words.
    /// The rejected claim is dropped here rather than never taken, which is
    /// what makes the check atomic — [`claim`](Self::claim) increments under
    /// the lock and reports what it found, so of two racing callers exactly
    /// one reads `0`.
    pub fn try_claim(self: Arc<Self>, discovery_id: &str) -> Option<RunningTurn> {
        let (turn, already_running) = self.claim(discovery_id);
        (already_running == 0).then_some(turn)
    }

    /// Mark a turn as running until the returned guard drops, and say how many
    /// were already running when it did.
    ///
    /// Taken by `Arc` rather than by reference because a claim outlives the
    /// scope that took it: an interview turn is claimed on the task that
    /// accepted the message and released on the spawned one that runs it, so a
    /// guard borrowing this would be a guard that could not cross the spawn.
    fn claim(self: Arc<Self>, discovery_id: &str) -> (RunningTurn, usize) {
        let already_running = {
            let mut counts = self.lock();
            let count = counts.entry(discovery_id.to_string()).or_insert(0);
            let already_running = *count;
            *count += 1;
            already_running
        };
        (
            RunningTurn {
                discovery_id: discovery_id.to_string(),
                turns: self,
            },
            already_running,
        )
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
///
/// Release is counted rather than a removal because
/// [`RunningTurns::try_claim`] refuses by dropping a claim it did take: an
/// unconditional removal would have the refused caller end the turn it was
/// refused for.
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
