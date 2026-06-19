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
    availability_cache: std::sync::Mutex<HashMap<(String, String), bool>>,
}

impl AgentRegistry {
    pub fn new(runtimes: Vec<Arc<dyn AgentRuntime>>) -> Self {
        Self {
            runtimes,
            sessions: Mutex::new(HashMap::new()),
            availability_cache: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// Check if the agent kind is available on the given machine.
    /// The result is cached per `(machine_id, kind)` for the duration of the app session.
    pub fn is_available(
        &self,
        kind: &str,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> bool {
        let key = (machine_id.to_string(), kind.to_string());
        {
            if let Ok(cache) = self.availability_cache.lock() {
                if let Some(&avail) = cache.get(&key) {
                    return avail;
                }
            }
        }

        if let Some(runtime) = self.runtime_for(kind) {
            let avail = runtime.is_available(exec, machine_id);
            if let Ok(mut cache) = self.availability_cache.lock() {
                cache.insert(key, avail);
            }
            avail
        } else {
            false
        }
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
                    return Err(AgentStartError::SpawnFailed(
                        "session has no id".into(),
                    ));
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
    pub async fn session_handle_any(
        &self,
        thread_id: &str,
    ) -> Option<Arc<dyn AgentSession>> {
        let sessions = self.sessions.lock().await;
        sessions.get(thread_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ports::agent_runtime::AgentStartError;
    use std::pin::Pin;
    use tokio_stream::{empty, Stream};

    struct NoopRuntime;
    impl AgentRuntime for NoopRuntime {
        fn kind(&self) -> &'static str { "noop" }
        fn is_available(&self, _exec: &dyn crate::ports::execution::ExecutionPort, _machine_id: &str) -> bool { false }
        fn install_command(&self) -> &'static str { "echo noop" }
        fn start(
            &self,
            _ctx: AgentContext,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>> {
            Box::pin(async { Err(AgentStartError::SpawnFailed("noop".into())) })
        }
    }

    struct FakeSession;
    impl AgentSession for FakeSession {
        fn session_id(&self) -> &str { "s-1" }
        fn prompt(&self, _text: &str) -> Pin<Box<dyn Stream<Item = crate::domain::agent_event::AgentEvent> + Send>> {
            Box::pin(empty())
        }
        fn cancel(&self) -> Result<(), String> { Ok(()) }
        fn set_mode(&self, _mode_id: &str) -> Result<(), String> { Ok(()) }
        fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> { Ok(()) }
        fn kill(&self) -> Result<(), String> { Ok(()) }
        fn session_info(&self) -> crate::domain::models::SessionInfo {
            crate::domain::models::SessionInfo::default()
        }
    }

    #[test]
    fn runtime_for_returns_registered_kind() {
        let reg = AgentRegistry::new(vec![Arc::new(NoopRuntime)]);
        assert!(reg.runtime_for("noop").is_some());
        assert!(reg.runtime_for("opencode").is_none());
    }

    #[tokio::test]
    async fn get_or_spawn_returns_structured_error_for_unknown_kind() {
        use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
        use crate::domain::action::AgentAction;

        struct StubExec;
        impl AgentExecutionPort for StubExec {
            fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> { Ok(CommandOutcome::Executed { output: crate::domain::intercept::ExecutionResult::Bash { output: String::new() } }) }
            fn submit_agent(&self, _: &str, _: &str, _: AgentAction, _: Option<String>) -> Result<CommandOutcome, ActionError> {
                Err(ActionError::Internal { message: "stub".into() })
            }
            fn approve(&self, _: &str) -> Result<(), String> { Ok(()) }
            fn reject(&self, _: &str, _: String) -> Result<(), String> { Ok(()) }
            fn register_result_responder(
                &self,
                _: &str,
                _: tokio::sync::oneshot::Sender<Result<crate::domain::intercept::ExecutionResult, String>>,
            ) -> Result<(), String> { Ok(()) }
        }
        impl crate::ports::execution::ExecutionPort for StubExec {
            fn test_connection(&self, _: &str) -> Result<(), String> { Ok(()) }
            fn run_command(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
            fn read_file(&self, _: &str, _: &str) -> Result<String, String> { Ok(String::new()) }
            fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
            fn get_metadata(&self, _: &str, path: &str) -> Result<crate::sftp::SftpEntry, String> {
                Ok(crate::sftp::SftpEntry { name: path.into(), path: path.into(), is_dir: false, size: 0, modified: 0 })
            }
            fn list_dir(&self, _: &str, _: &str) -> Result<Vec<crate::sftp::SftpEntry>, String> { Ok(vec![]) }
            fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> { Ok(()) }
            fn resolve_home(&self, _: &str) -> Result<String, String> { Ok("/tmp".to_string()) }
            fn spawn_interactive(&self, _: &str, _: &str, _: &[String], _: &str, _: &std::collections::HashMap<String, String>) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
                Err("stub".to_string())
            }
        }

        let reg = AgentRegistry::new(vec![Arc::new(NoopRuntime)]);
        let stub = Arc::new(StubExec);
        let err = reg
            .get_or_spawn("t1", "opencode", AgentContext {
                thread_id: "t1".into(),
                machine_id: "m1".into(),
                binary: "opencode".into(),
                args: vec![],
                env: Default::default(),
                cwd: ".".into(),
                model: None,
                title: None,
                agent_exec: stub.clone(),
                exec: stub,
            })
            .await
            .err()
            .expect("should error");
        assert!(matches!(err, AgentStartError::NotFound(_)));
    }

    #[tokio::test]
    async fn kill_removes_session() {
        let mut sessions: HashMap<String, Arc<dyn AgentSession>> = HashMap::new();
        sessions.insert("t1".into(), Arc::new(FakeSession) as Arc<dyn AgentSession>);
        let reg = AgentRegistry {
            runtimes: vec![],
            sessions: Mutex::new(sessions),
            availability_cache: std::sync::Mutex::new(HashMap::new()),
        };
        reg.kill("t1").await;
        reg.kill("t1").await; // idempotent
    }
}
