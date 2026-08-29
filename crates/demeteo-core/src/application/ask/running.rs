//! Which Ask threads have a turn running *in this process*.
//!
//! Same mechanism as
//! [`discovery::running::RunningTurns`](crate::application::discovery::running::RunningTurns) —
//! see that module for why nothing durable holds this claim, why the claim is
//! exclusive to acquire but counted to release, and why a second turn on one
//! thread ends the first rather than queuing behind it. This is a fully
//! independent instance: an Ask thread and a Discovery never share a claim
//! space, even if both happened to reuse the same id string, and neither
//! module may import the other's claim type.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::domain::ids::AskThreadId;

/// What a caller is told when the Ask thread already has a turn.
pub const ALREADY_RUNNING: &str = "This Ask thread is already working — a turn is under way. \
                                   Wait for it to finish before starting another.";

#[derive(Default)]
pub struct RunningTurns {
    counts: Mutex<HashMap<String, usize>>,
}

impl RunningTurns {
    /// Take the turn only if this Ask thread has none, which is the only way in.
    pub fn try_claim(self: Arc<Self>, thread_id: &str) -> Option<RunningTurn> {
        let (turn, already_running) = self.claim(thread_id);
        (already_running == 0).then_some(turn)
    }

    /// Mark a turn as running until the returned guard drops, and say how many
    /// were already running when it did.
    fn claim(self: Arc<Self>, thread_id: &str) -> (RunningTurn, usize) {
        let already_running = {
            let mut counts = self.lock();
            let count = counts.entry(thread_id.to_string()).or_insert(0);
            let already_running = *count;
            *count += 1;
            already_running
        };
        (
            RunningTurn {
                thread_id: thread_id.to_string(),
                turns: self,
            },
            already_running,
        )
    }

    pub fn running(&self, thread_id: &AskThreadId) -> bool {
        self.lock().contains_key(thread_id.as_str())
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, usize>> {
        self.counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One turn's claim for an Ask thread, held for as long as this lives.
pub struct RunningTurn {
    turns: Arc<RunningTurns>,
    thread_id: String,
}

impl Drop for RunningTurn {
    fn drop(&mut self) {
        let mut counts = self.turns.lock();
        match counts.get_mut(&self.thread_id) {
            Some(count) if *count > 1 => *count -= 1,
            _ => {
                counts.remove(&self.thread_id);
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/application/ask/running.rs"]
mod tests;
