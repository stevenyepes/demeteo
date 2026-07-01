//! Shared test stubs for unit-level agent arg tests.
//!
//! `AgentContext` requires two trait-object ports
//! (`AgentExecutionPort`, `ExecutionPort`). The unit tests for
//! `build_*_args` only need *any* value of these traits — they
//! don't exercise the runtime path. The stubs here return the
//! minimum the trait demands (no-ops) so tests can construct a
//! context purely to call `build_*_args(ctx, sid)`.
//!
//! Loaded exactly once from `src/adapters/agent/mod.rs` so the
//! `clippy::duplicate-mod` lint stays clean (each test file in
//! `tests/infrastructure/agent/*.rs` reuses these via the
//! crate-path `use crate::adapters::agent::test_stubs::...`).
//!
//! NOTE: keep this in sync with `FakeAgentExec` / `FakeExec` in
//! `tests/e2e/step_executor.rs`. We duplicate rather than share
//! because the e2e stubs are heavier (they wire up actual
//! intercept_id routing).

use crate::domain::action::AgentAction;
use crate::domain::intercept::ExecutionResult;
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::sftp::SftpEntry;

pub struct StubAgentExec;

#[async_trait::async_trait]
impl AgentExecutionPort for StubAgentExec {
    async fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
        Ok(CommandOutcome::Executed {
            output: ExecutionResult::Bash {
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
        Err(ActionError::internal("stub"))
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
        _: tokio::sync::oneshot::Sender<Result<ExecutionResult, String>>,
    ) -> Result<(), String> {
        Ok(())
    }
}

pub struct StubExec;

#[async_trait::async_trait]
impl ExecutionPort for StubExec {
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
    async fn get_metadata(&self, _: &str, path: &str) -> Result<SftpEntry, String> {
        Ok(SftpEntry {
            name: path.into(),
            path: path.into(),
            is_dir: false,
            size: 0,
            modified: 0,
        })
    }
    async fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
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
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        Err("stub".to_string())
    }
}
