use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};

/// Thread-id-keyed registry of live agent sessions. Owns the lazy lifecycle:
/// sessions are created on the first directive, torn down on idle timeout /
/// thread delete / app shutdown. Phase 7a only registers a `NoopRuntime` so
/// the wiring compiles and the dispatcher has something to return a
/// structured `AgentStartError::NotFound` from.
pub struct AgentRegistry {
    runtimes: Vec<Arc<dyn AgentRuntime>>,
    sessions: Mutex<HashMap<String, Arc<dyn AgentSession>>>,
    availability_cache: tokio::sync::Mutex<HashMap<(String, String), bool>>,
}

impl AgentRegistry {
    pub fn new(runtimes: Vec<Arc<dyn AgentRuntime>>) -> Self {
        Self {
            runtimes,
            sessions: Mutex::new(HashMap::new()),
            availability_cache: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Check if the agent kind is available on the given machine.
    /// The result is cached per `(machine_id, kind)` for the duration of the app session.
    ///
    /// When `force` is true the cache is bypassed and the result of the
    /// fresh probe is written back into the cache. The settings page's
    /// "Re-check agent availability" button calls with `force = true` so
    /// that installing a binary mid-session is reflected immediately,
    /// instead of waiting for an app restart.
    pub async fn is_available(
        &self,
        kind: &str,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
        force: bool,
    ) -> bool {
        let key = (machine_id.to_string(), kind.to_string());
        if !force {
            let cache = self.availability_cache.lock().await;
            if let Some(&avail) = cache.get(&key) {
                return avail;
            }
        }

        let runtime = match self.runtime_for(kind) {
            Some(r) => r,
            None => {
                if !force {
                    let mut cache = self.availability_cache.lock().await;
                    cache.insert(key, false);
                }
                return false;
            }
        };
        let avail = runtime.is_available(exec, machine_id).await;
        let mut cache = self.availability_cache.lock().await;
        cache.insert(key, avail);
        avail
    }

    /// Resolve which runtime owns a given `kind`. The lookup is exact; v1
    /// has two runtimes (`opencode`, `hermes`) and the picker hands the
    /// selected `kind` straight through.
    pub fn runtime_for(&self, kind: &str) -> Option<Arc<dyn AgentRuntime>> {
        self.runtimes.iter().find(|r| r.kind() == kind).cloned()
    }

    pub fn runtimes(&self) -> &[Arc<dyn AgentRuntime>] {
        &self.runtimes
    }

    pub async fn get_or_spawn(
        &self,
        thread_id: &str,
        kind: &str,
        ctx: AgentContext,
    ) -> Result<Arc<dyn AgentSession>, AgentStartError> {
        {
            let sessions = self.sessions.lock().await;
            if let Some(s) = sessions.get(thread_id) {
                if s.session_id().is_empty() {
                    return Err(AgentStartError::SpawnFailed("session has no id".into()));
                }
                return Ok(s.clone());
            }
        }

        let runtime = self
            .runtime_for(kind)
            .ok_or_else(|| AgentStartError::NotFound(kind.into()))?;
        let session = runtime.start(ctx).await?;
        let mut sessions = self.sessions.lock().await;
        sessions.insert(thread_id.to_string(), session.clone());
        Ok(session)
    }

    pub async fn kill(&self, thread_id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(s) = sessions.remove(thread_id) {
            // Force-kill the session's transport so the agent process
            // is actually reaped even if other Arc references exist
            // (e.g. the old driver loop). Removing from the map alone
            // would leave the transport alive until all Arcs drop.
            let _ = s.kill();
        }
    }

    pub async fn kill_all(&self) {
        let mut sessions = self.sessions.lock().await;
        sessions.clear();
    }

    /// Look up the live session for `(thread_id, kind)`. Returns the
    /// `Arc<dyn AgentSession>` if one exists. Used by `agent_start`
    /// after a successful spawn to confirm the session is in the
    /// registry (and to enable future Phase 7e cross-transport swaps).
    pub async fn session_handle(
        &self,
        thread_id: &str,
        _kind: &str,
    ) -> Option<Arc<dyn AgentSession>> {
        let sessions = self.sessions.lock().await;
        sessions.get(thread_id).cloned()
    }

    /// Same as `session_handle` but ignores the kind — we only store
    /// one session per thread. Used by `agent_cancel` which doesn't
    /// know which adapter is in play.
    pub async fn session_handle_any(&self, thread_id: &str) -> Option<Arc<dyn AgentSession>> {
        let sessions = self.sessions.lock().await;
        sessions.get(thread_id).cloned()
    }

    /// Read the cumulative input+output token count from the live
    /// session for `thread_id` (if any). Used by the driver's
    /// context-window watchdog — returns `0` when no session is
    /// registered (the watchdog treats that as "no data, skip
    /// check").
    pub async fn cumulative_tokens(&self, thread_id: &str) -> Result<u64, String> {
        let sessions = self.sessions.lock().await;
        match sessions.get(thread_id) {
            Some(s) => Ok(s.cumulative_tokens()),
            None => Ok(0),
        }
    }

    /// Whether the live session for `thread_id` is still alive (its
    /// underlying agent process / SSH channel hasn't exited). Used
    /// by the driver's dead-session fallback before re-spawning.
    pub async fn is_session_alive(&self, thread_id: &str) -> bool {
        let sessions = self.sessions.lock().await;
        match sessions.get(thread_id) {
            Some(s) => s.is_alive(),
            None => false,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/registry.rs"]
mod tests;
