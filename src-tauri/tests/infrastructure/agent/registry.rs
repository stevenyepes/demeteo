use super::*;
use crate::ports::agent_runtime::AgentStartError;
use std::pin::Pin;
use tokio_stream::{empty, Stream};

struct NoopRuntime;
#[async_trait::async_trait]
impl AgentRuntime for NoopRuntime {
    fn kind(&self) -> &'static str {
        "noop"
    }
    async fn is_available(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> bool {
        false
    }
    fn install_command(&self) -> &'static str {
        "echo noop"
    }
    fn start(
        &self,
        _ctx: AgentContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(AgentStartError::SpawnFailed("noop".into())) })
    }
}

struct FakeSession;
impl AgentSession for FakeSession {
    fn session_id(&self) -> &str {
        "s-1"
    }
    fn prompt(
        &self,
        _text: &str,
    ) -> Pin<Box<dyn Stream<Item = crate::domain::agent_event::AgentEvent> + Send>> {
        Box::pin(empty())
    }
    fn cancel(&self) -> Result<(), String> {
        Ok(())
    }
    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        Ok(())
    }
    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }
    fn kill(&self) -> Result<(), String> {
        Ok(())
    }
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
    use crate::domain::action::AgentAction;
    use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};

    struct StubExec;
    #[async_trait::async_trait]
    impl AgentExecutionPort for StubExec {
        async fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
            Ok(CommandOutcome::Executed {
                output: crate::domain::intercept::ExecutionResult::Bash {
                    output: String::new(),
                },
            })
        }
        async fn submit_agent(
            &self,
            _: &str,
            _: &str,
            _: AgentAction,
            _: Option<String>,
        ) -> Result<CommandOutcome, ActionError> {
            Err(ActionError::Internal {
                message: "stub".into(),
            })
        }
        async fn approve(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn reject(&self, _: &str, _: String) -> Result<(), String> {
            Ok(())
        }
        async fn register_result_responder(
            &self,
            _: &str,
            _: tokio::sync::oneshot::Sender<
                Result<crate::domain::intercept::ExecutionResult, String>,
            >,
        ) -> Result<(), String> {
            Ok(())
        }
    }
    #[async_trait::async_trait]
    impl crate::ports::execution::ExecutionPort for StubExec {
        async fn test_connection(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
            Ok(String::new())
        }
        async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn get_metadata(
            &self,
            _: &str,
            path: &str,
        ) -> Result<crate::sftp::SftpEntry, String> {
            Ok(crate::sftp::SftpEntry {
                name: path.into(),
                path: path.into(),
                is_dir: false,
                size: 0,
                modified: 0,
            })
        }
        async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<crate::sftp::SftpEntry>, String> {
            Ok(vec![])
        }
        async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        async fn resolve_home(&self, _: &str) -> Result<String, String> {
            Ok("/tmp".to_string())
        }
        fn spawn_interactive(
            &self,
            _: &str,
            _: &str,
            _: &[String],
            _: &str,
            _: &std::collections::HashMap<String, String>,
        ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
            Err("stub".to_string())
        }
    }

    let reg = AgentRegistry::new(vec![Arc::new(NoopRuntime)]);
    let stub = Arc::new(StubExec);
    let err = reg
        .get_or_spawn(
            "t1",
            "opencode",
            AgentContext {
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
                permissions: crate::domain::permission::PermissionProfile::all_allow(),
            },
        )
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
        availability_cache: tokio::sync::Mutex::new(HashMap::new()),
    };
    reg.kill("t1").await;
    reg.kill("t1").await;
}

/// Runtime that lets the test flip the `is_available` answer between
/// probes. Counts how many times `is_available` was called so the cache
/// behavior can be asserted from the call count alone.
struct FlippableRuntime {
    state: tokio::sync::Mutex<bool>,
    calls: tokio::sync::Mutex<u32>,
}

impl FlippableRuntime {
    fn new(initial: bool) -> Arc<Self> {
        Arc::new(Self {
            state: tokio::sync::Mutex::new(initial),
            calls: tokio::sync::Mutex::new(0),
        })
    }
    async fn flip(&self) {
        let mut s = self.state.lock().await;
        *s = !*s;
    }
    async fn calls(&self) -> u32 {
        *self.calls.lock().await
    }
}

#[async_trait::async_trait]
impl AgentRuntime for FlippableRuntime {
    fn kind(&self) -> &'static str {
        "flippable"
    }
    async fn is_available(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> bool {
        let mut c = self.calls.lock().await;
        *c += 1;
        *self.state.lock().await
    }
    fn install_command(&self) -> &'static str {
        "echo flippable"
    }
    fn start(
        &self,
        _ctx: AgentContext,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>>
                + Send
                + '_,
        >,
    > {
        Box::pin(async { Err(AgentStartError::SpawnFailed("flippable".into())) })
    }
}

/// When the user's installation toggles the binary on disk mid-session,
/// the next click of the "Re-check availability" button must return the
/// fresh value rather than the cached `false` from the previous probe.
#[tokio::test]
async fn is_available_force_bypasses_cache() {
    let rt = FlippableRuntime::new(false);
    let reg = AgentRegistry::new(vec![rt.clone()]);

    // 1. Initial probe: binary missing. Should be cached.
    let stub: Arc<dyn crate::ports::execution::ExecutionPort> = {
        struct NoopExec;
        #[async_trait::async_trait]
        impl crate::ports::execution::ExecutionPort for NoopExec {
            async fn test_connection(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            async fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
                Ok(String::new())
            }
            async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
                Ok(String::new())
            }
            async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
                Ok(())
            }
            async fn get_metadata(
                &self,
                _: &str,
                path: &str,
            ) -> Result<crate::sftp::SftpEntry, String> {
                Ok(crate::sftp::SftpEntry {
                    name: path.into(),
                    path: path.into(),
                    is_dir: false,
                    size: 0,
                    modified: 0,
                })
            }
            async fn list_dir(
                &self,
                _: &str,
                _: &str,
            ) -> Result<Vec<crate::sftp::SftpEntry>, String> {
                Ok(vec![])
            }
            async fn setup_worktree(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<(), String> {
                Ok(())
            }
            async fn resolve_home(&self, _: &str) -> Result<String, String> {
                Ok("/tmp".into())
            }
            fn spawn_interactive(
                &self,
                _: &str,
                _: &str,
                _: &[String],
                _: &str,
                _: &std::collections::HashMap<String, String>,
            ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
                Err("noop".into())
            }
        }
        Arc::new(NoopExec)
    };

    assert!(
        !reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert_eq!(rt.calls().await, 1, "first call must probe");

    // 2. Cached: subsequent non-forced calls must NOT re-probe.
    assert!(
        !reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert!(
        !reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert_eq!(rt.calls().await, 1, "non-forced calls must hit the cache");

    // 3. The user installs the binary. Flip the underlying runtime's
    //    answer to `true` and force a re-probe via the refresh button.
    rt.flip().await;
    assert!(
        reg.is_available("flippable", stub.as_ref(), "m1", true)
            .await
    );
    assert_eq!(rt.calls().await, 2, "forced call must re-probe");

    // 4. The cache now reflects the fresh value.
    assert!(
        reg.is_available("flippable", stub.as_ref(), "m1", false)
            .await
    );
    assert_eq!(rt.calls().await, 2, "fresh value must be cached");
}
