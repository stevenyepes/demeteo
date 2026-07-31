//! The doubles the e2e suite runs against, and the wiring that assembles a
//! real `DagStepExecutor` around them.
//!
//! `StrictExec` answers only what a test scripted and errors on everything
//! else. Its predecessor answered `Ok("")` to every command, which meant every
//! assertion over git's output was really an assertion over an empty string
//! nobody had produced — the shape AGENTS.md §7 names as what makes a suite
//! unable to fail.

use std::sync::Arc;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::adapters::step_executor::DagStepExecutor;
use crate::domain::action::AgentAction;
use crate::domain::intercept::ExecutionResult;
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::notification::{DomainEvent, NotificationPort};

pub(super) struct FakeNotif;
impl NotificationPort for FakeNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// A `NotificationPort` that records every emitted event, so a test can
/// assert on the `BootstrapProgress` / `FeatureStatusChanged` sequence a
/// bootstrap produces.
#[derive(Default)]
pub(super) struct CapturingNotif {
    pub(super) events: std::sync::Mutex<Vec<DomainEvent>>,
}
impl NotificationPort for CapturingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

pub(super) struct FakeAgentExec;
#[async_trait::async_trait]
impl AgentExecutionPort for FakeAgentExec {
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

/// Helper: build a fully wired `DagStepExecutor` backed by an isolated
/// on-disk SQLite DB. Used by every guard test below — keeps the
/// boilerplate out of each test body. Returns `(executor, db, temp_dir)`
/// so callers can poke at the DB directly when needed.
pub(super) async fn build_test_executor(
    label: &str,
) -> (DagStepExecutor, Arc<SqliteAdapter>, std::path::PathBuf) {
    build_test_executor_with_notif(label, Arc::new(FakeNotif)).await
}

pub(super) async fn build_test_executor_with_notif(
    label: &str,
    notif: Arc<dyn NotificationPort>,
) -> (DagStepExecutor, Arc<SqliteAdapter>, std::path::PathBuf) {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_guard_{}_{}",
        label,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let conn = crate::db::init_db(temp_dir.clone()).expect("init_db failed");
    let db = Arc::new(SqliteAdapter::new(conn).unwrap());
    let registry = Arc::new(AgentRegistry::new(vec![]));
    let agent_exec = Arc::new(FakeAgentExec);
    let exec = Arc::new(ScriptedExec::new(&[]));
    let artifacts: Arc<dyn crate::ports::artifact_store::ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );
    let attachments: Arc<dyn crate::ports::attachment_store::AttachmentStore> =
        Arc::new(crate::adapters::attachment_store::fs::FsAttachmentStore::new(temp_dir.clone()));

    let merge_executor: Arc<dyn crate::ports::merge::MergeExecutor> = {
        let git_ops =
            crate::adapters::worktree::git_ops::GitOpsHelper::new(db.clone(), exec.clone());
        Arc::new(crate::adapters::merge::SqliteMergeExecutor::new(
            db.clone(),
            git_ops,
            exec.clone(),
            temp_dir.clone(),
        ))
    };

    let memory_llm: Arc<dyn crate::ports::memory_llm::MemoryLlmPort> =
        Arc::new(crate::adapters::memory_llm::ReqwestMemoryLlmAdapter::new());
    let pricing: Arc<dyn crate::ports::pricing::PricingTable> =
        Arc::new(crate::adapters::pricing::HardcodedPricingTable::new());
    let executor = DagStepExecutor::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(), // memory
        db.clone(), // signals
        memory_llm,
        registry,
        notif,
        db.clone(), // notifications
        agent_exec,
        exec,
        merge_executor,
        db.clone(), // subtask_runs — SqliteAdapter implements the port
        db.clone(), // sequence_resume — SqliteAdapter implements the port
        artifacts,
        attachments,
        db.clone(), // attachment_json — SqliteAdapter implements both ports
        temp_dir.clone(),
        pricing,
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
    );
    (executor, db, temp_dir)
}
