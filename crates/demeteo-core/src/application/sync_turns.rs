//! Which features have a sync running *in this process*, and the only thing
//! that can answer it.
//!
//! Half of [`sync_liveness`](crate::domain::sync_session::sync_liveness)'s
//! input. The other half — a workflow's own `sync` node — is visible in the
//! feature's status, because a driver holds the run; an out-of-band merge or
//! resolution started from the Sync pane runs on a feature no driver owns, so
//! nothing durable says it is happening and nothing may be written down that
//! does. A row claiming a live turn would outlive the process that made it, and
//! the next start would then refuse to correct a session whose worker died with
//! it. Forgetting on restart is the behaviour, not a limitation of it.
//!
//! An entry is also the mutual exclusion between two turns on one feature: a
//! second resolution put in the same worktree is the thing
//! [`user_may_intervene`](crate::domain::sync_session::user_may_intervene)
//! exists to prevent, and the session row cannot refuse it alone.

use std::collections::HashMap;
use std::sync::Mutex;

use tokio::sync::watch;

/// The sync work this process is running, keyed by feature id.
#[derive(Default)]
pub struct SyncTurns {
    claims: Mutex<HashMap<String, Option<watch::Sender<bool>>>>,
}

impl SyncTurns {
    /// Take the feature's slot, or refuse because something already holds it.
    ///
    /// `cancel` is the turn's own Stop channel where it has one. A merge does
    /// not: `sync_feature_with_upstream` runs git to completion and there is
    /// nothing to signal, but it still owns the worktree for the duration and
    /// so still has to be visible here.
    ///
    /// The slot comes back when the guard drops, and only that way. Released by
    /// hand it survived every `?` between the claim and the call that gave it
    /// back, and a leaked entry does not expire: nothing sweeps this map, so the
    /// feature's next sync, its next resolution and — since the entry became
    /// half of [`sync_liveness`](crate::domain::sync_session::sync_liveness) —
    /// every intervention on the session are all refused for the life of the
    /// process. Restarting the app was the only cure.
    pub fn claim(
        &self,
        feature_id: &str,
        cancel: Option<watch::Sender<bool>>,
    ) -> Option<SyncTurn<'_>> {
        let mut claims = self.lock();
        if claims.contains_key(feature_id) {
            return None;
        }
        claims.insert(feature_id.to_string(), cancel);
        Some(SyncTurn {
            turns: self,
            feature_id: feature_id.to_string(),
        })
    }

    pub fn claimed(&self, feature_id: &str) -> bool {
        self.lock().contains_key(feature_id)
    }

    /// Ask whatever holds the slot to stop, if it can be asked.
    pub fn cancel(&self, feature_id: &str) {
        if let Some(Some(tx)) = self.lock().get(feature_id) {
            let _ = tx.send(true);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<watch::Sender<bool>>>> {
        self.claims
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// One feature's slot, held for as long as this lives.
///
/// Whether the turn returned, returned early through a `?`, panicked or had its
/// future dropped, the slot is given back — which is the difference between a
/// crashed resolver the user can still abort and a session frozen `Live` until
/// the app restarts.
pub struct SyncTurn<'a> {
    turns: &'a SyncTurns,
    feature_id: String,
}

impl Drop for SyncTurn<'_> {
    fn drop(&mut self) {
        self.turns.lock().remove(&self.feature_id);
    }
}

#[cfg(test)]
#[path = "../../tests/application/sync_turns.rs"]
mod tests;
