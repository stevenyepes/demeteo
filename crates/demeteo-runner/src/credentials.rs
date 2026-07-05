//! In-memory, run-scoped git-provider credential store
//! (docs/REMOTE_EXECUTION_PLAN.md M4.2, docs/REMOTE_EXECUTION.md §6.2).
//!
//! Holds the PAT the laptop injects via `inject_credentials` keyed by
//! `run_id`. Deliberately the *only* place this process holds a git
//! secret: never written to the runner's SQLite, artifacts, git config,
//! or logs. Entries are removed the moment a run reaches a terminal
//! state (success, failure, or cancel) — a compromised *idle* runner
//! therefore has no git secret to steal.

use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Default)]
pub struct CredentialStore {
    pats: Mutex<HashMap<String, String>>,
}

impl CredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Re-supplying a PAT for a run already holding one overwrites it —
    /// this is the "re-inject after a runner restart" path (§6.2), not
    /// just first-injection.
    pub fn insert(&self, run_id: &str, pat: String) {
        self.pats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(run_id.to_string(), pat);
    }

    pub fn get(&self, run_id: &str) -> Option<String> {
        self.pats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(run_id)
            .cloned()
    }

    /// Wipe the credential for a run. Called on every terminal state
    /// (completed / failed / cancelled / pr_ready) so the PAT's
    /// in-memory lifetime never outlives the run it was injected for.
    pub fn remove(&self, run_id: &str) {
        self.pats
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(run_id);
    }
}
