use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::domain::models::{Availability, EffortLevel, WindowsAgentShell};
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};

/// Thread-id-keyed registry of live agent sessions. Owns the lazy lifecycle:
/// sessions are created on the first directive, torn down on idle timeout /
/// thread delete / app shutdown. Phase 7a only registers a `NoopRuntime` so
/// the wiring compiles and the dispatcher has something to return a
/// structured `AgentStartError::NotFound` from.
pub struct AgentRegistry {
    runtimes: Vec<Arc<dyn AgentRuntime>>,
    sessions: Mutex<HashMap<String, Arc<dyn AgentSession>>>,
    availability_cache: tokio::sync::Mutex<HashMap<(String, String), Availability>>,
}

impl AgentRegistry {
    pub fn new(runtimes: Vec<Arc<dyn AgentRuntime>>) -> Self {
        Self {
            runtimes,
            sessions: Mutex::new(HashMap::new()),
            availability_cache: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Whether the agent kind is known to be installed on the given machine.
    /// The lossy read of [`Self::availability`], for callers about to run the
    /// agent: an unanswered probe counts as "no".
    pub async fn is_available(
        &self,
        kind: &str,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
        force: bool,
    ) -> bool {
        self.availability(kind, exec, machine_id, force)
            .await
            .is_installed()
    }

    /// Probe whether the agent kind is installed on the given machine.
    /// A *conclusive* result is cached per `(machine_id, kind)` for the
    /// duration of the app session; [`Availability::Unknown`] is deliberately
    /// not cached, so one unreachable moment doesn't pin every agent on that
    /// machine to "missing" until the app restarts.
    ///
    /// When `force` is true the cache is bypassed and the result of the
    /// fresh probe is written back into the cache. The settings page's
    /// "Re-check agent availability" button calls with `force = true` so
    /// that installing a binary mid-session is reflected immediately,
    /// instead of waiting for an app restart.
    pub async fn availability(
        &self,
        kind: &str,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
        force: bool,
    ) -> Availability {
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
                // Nothing to probe and nothing that could change: an
                // unregistered kind is absent by definition.
                if !force {
                    let mut cache = self.availability_cache.lock().await;
                    cache.insert(key, Availability::Missing);
                }
                return Availability::Missing;
            }
        };
        let avail = runtime.availability(exec, machine_id).await;
        if avail.is_conclusive() {
            let mut cache = self.availability_cache.lock().await;
            cache.insert(key, avail);
        }
        avail
    }

    /// Probe `kinds` on one machine, in order, and pair each with its answer.
    ///
    /// Stops probing the moment a kind comes back [`Availability::Unknown`]
    /// and reports the rest as `Unknown` too. That result says the *machine*
    /// did not answer, which is not a fact about the kind — asking the next
    /// one buys the same answer at the same price. The price is why this
    /// exists: an unreachable host costs a 5s TCP timeout times three retry
    /// attempts, and since an inconclusive answer is deliberately not cached,
    /// a caller looping over every kind pays that per kind, every time it
    /// looks. One machine, one bill.
    ///
    /// Every entry is still returned in the order given, so a caller can zip
    /// it against its own list.
    pub async fn availability_of<'k>(
        &self,
        kinds: &[&'k str],
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
        force: bool,
    ) -> Vec<(&'k str, Availability)> {
        let mut out = Vec::with_capacity(kinds.len());
        let mut unreachable = false;
        for kind in kinds {
            let avail = if unreachable {
                Availability::Unknown
            } else {
                let probed = self.availability(kind, exec, machine_id, force).await;
                unreachable = !probed.is_conclusive();
                probed
            };
            out.push((*kind, avail));
        }
        out
    }

    /// Resolve which runtime owns a given `kind`. The lookup is exact; v1
    /// has two runtimes (`opencode`, `hermes`) and the picker hands the
    /// selected `kind` straight through.
    pub fn runtime_for(&self, kind: &str) -> Option<Arc<dyn AgentRuntime>> {
        self.runtimes.iter().find(|r| r.kind() == kind).cloned()
    }

    /// Returns the default model name for `kind`, or `None` when the runtime
    /// doesn't have a statically knowable default. Used to seed
    /// `UsageAccumulator` for pricing-table fallback cost calculation.
    pub fn default_model_for(&self, kind: &str) -> Option<String> {
        self.runtime_for(kind)?.default_model()
    }

    /// The effort levels `kind` accepts per invocation, in ladder order.
    /// Empty for an agent with no effort control (hermes) *and* for an
    /// unknown kind — in both cases there is no level a caller could
    /// legitimately offer. Drives the UI picker so it can't offer one the
    /// agent would ignore.
    pub fn effort_levels_for(&self, kind: &str) -> &'static [EffortLevel] {
        self.runtime_for(kind)
            .map(|r| r.capabilities().effort_levels)
            .unwrap_or(&[])
    }

    /// The interpreter `kind` runs agent-authored commands under on Windows.
    ///
    /// An unrecognised kind answers
    /// [`Unknown`](crate::domain::models::WindowsAgentShell::Unknown) rather
    /// than a default, on the same reasoning as the variant itself: a legacy
    /// stored kind is precisely the case where nobody has checked.
    pub fn windows_agent_shell_for(&self, kind: &str) -> WindowsAgentShell {
        self.runtime_for(kind)
            .map(|r| r.capabilities().windows_agent_shell)
            .unwrap_or(WindowsAgentShell::Unknown)
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

    /// Kill every registered session whose key belongs to `f_id`.
    ///
    /// Agent-step sessions are now keyed by `{f_id}::{fingerprint}`
    /// (see `ExecutionDriver::agent_session_key`) rather than the bare
    /// feature id, so a single pipeline run can leave more than one
    /// live entry behind — one per distinct permission-profile/model
    /// combination it visited. The old single-key `kill(f_id)` only
    /// ever tore down the *last* one. Call this at every true
    /// pipeline-terminal point (success, failure, cancellation) so
    /// earlier same-feature segments don't leak for the life of the
    /// app. Also catches the verifier/planner/subtask sessions, which
    /// use `{f_id}-...` suffixes and normally self-clean — a harmless,
    /// defensive superset.
    pub async fn kill_all_for_feature(&self, f_id: &str) {
        let mut sessions = self.sessions.lock().await;
        let dead_keys: Vec<String> = sessions
            .keys()
            .filter(|k| {
                k.as_str() == f_id
                    || k.starts_with(&format!("{f_id}::"))
                    || k.starts_with(&format!("{f_id}-"))
            })
            .cloned()
            .collect();
        for key in dead_keys {
            if let Some(s) = sessions.remove(&key) {
                let _ = s.kill();
            }
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
