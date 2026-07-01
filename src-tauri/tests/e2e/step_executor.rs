use std::sync::Arc;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::DagStepExecutor;
use crate::domain::action::AgentAction;
use crate::domain::ids::{
    FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId,
};
use crate::domain::intercept::ExecutionResult;
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::paths;
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::{GatePresenter, StepExecutor};
use crate::sftp::SftpEntry;

struct FakeNotif;
impl NotificationPort for FakeNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

struct FakeAgentExec;
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

struct FakeExec;
#[async_trait::async_trait]
impl ExecutionPort for FakeExec {
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

#[tokio::test]
async fn test_executor_instantiation_and_cancel() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_exec_instantiation_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let conn = crate::db::init_db(temp_dir.clone()).expect("init_db failed");
    let db = Arc::new(SqliteAdapter::new(conn).unwrap());
    let registry = Arc::new(AgentRegistry::new(vec![]));
    let notif = Arc::new(FakeNotif);
    let agent_exec = Arc::new(FakeAgentExec);
    let exec = Arc::new(FakeExec);
    let artifacts: Arc<dyn crate::ports::artifact_store::ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );
    let attachments: Arc<dyn crate::ports::attachment_store::AttachmentStore> =
        Arc::new(crate::adapters::attachment_store::fs::FsAttachmentStore::new(temp_dir.clone()));

    let merge_executor: Arc<dyn crate::ports::merge::MergeExecutor> = {
        // The git_ops helper needs an `AppSettingsRepository`; the
        // adapter also implements that port so we can hand a
        // second clone of the same Arc to the helper. The merge
        // executor itself only needs the SQLite connection.
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
        artifacts,
        attachments,
        db.clone(), // attachment_json — SqliteAdapter implements both ports
        temp_dir.clone(),
        pricing,
    );

    let cancel_res = executor.feature_cancel("f-nonexistent").await;
    assert!(cancel_res.is_ok());

    let _ = std::fs::remove_dir_all(temp_dir);
}

#[tokio::test]
async fn test_executor_gate_decide() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_exec_gate_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let conn = crate::db::init_db(temp_dir.clone()).expect("init_db failed");
    let db = Arc::new(SqliteAdapter::new(conn).unwrap());
    let registry = Arc::new(AgentRegistry::new(vec![]));
    let notif = Arc::new(FakeNotif);
    let agent_exec = Arc::new(FakeAgentExec);
    let exec = Arc::new(FakeExec);
    let artifacts: Arc<dyn crate::ports::artifact_store::ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );
    let attachments: Arc<dyn crate::ports::attachment_store::AttachmentStore> =
        Arc::new(crate::adapters::attachment_store::fs::FsAttachmentStore::new(temp_dir.clone()));

    let merge_executor: Arc<dyn crate::ports::merge::MergeExecutor> = {
        // The git_ops helper needs an `AppSettingsRepository`; the
        // adapter also implements that port so we can hand a
        // second clone of the same Arc to the helper. The merge
        // executor itself only needs the SQLite connection.
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
        artifacts,
        attachments,
        db.clone(), // attachment_json — SqliteAdapter implements both ports
        temp_dir.clone(),
        pricing,
    );

    let waiter = crate::adapters::step_executor::gate_waiter::GateWaiter::new();
    executor
        .gate_waiters()
        .lock()
        .unwrap()
        .insert("se-1".to_string(), waiter.clone());

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;
    let gates: &dyn GateRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-1"),
            name: "test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();

    features
        .add(Feature {
            id: FeatureId::from("f-1"),
            project_id: ProjectId::from("p-1"),
            workflow_id: Some(WorkflowId::from("w-1")),
            title: "test feature".to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
        })
        .unwrap();

    features
        .step_create(StepExecution {
            id: StepExecutionId::from("se-1"),
            feature_id: FeatureId::from("f-1"),
            step_id: StepId::from("step-1"),
            step_index: 0,
            step_kind: "gate".to_string(),
            status: "awaiting_gate".to_string(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            artifact_path: None,
            artifact_paths: Vec::new(),
            error_message: None,
            iteration_count: 0,
            created_at: now,
            updated_at: now,
        })
        .unwrap();

    gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-se-1"),
            step_execution_id: StepExecutionId::from("se-1"),
            decision: None,
            feedback: None,
            created_at: now,
        })
        .unwrap();

    let decide_res = executor
        .gate_decide("se-1", "approve", Some("looks good"))
        .await;
    assert!(decide_res.is_ok());

    let decision = waiter.wait().await.expect("waiter should deliver");
    assert_eq!(decision.decision.as_deref(), Some("approve"));
    assert_eq!(decision.feedback.as_deref(), Some("looks good"));

    // Idempotency: a re-delivery of the same decision should not change
    // the DB row (no second row, decision unchanged).
    executor
        .gate_decide("se-1", "approve", Some("looks good"))
        .await
        .unwrap();
    let latest = gates
        .latest_for_step(&StepExecutionId::from("se-1".to_string()))
        .unwrap()
        .unwrap();
    assert_eq!(latest.decision.as_deref(), Some("approve"));

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The bug the user reported: a `gate_decide` arrives after the driver
/// has died (app restart, panic, race). With the old code, the
/// oneshot::Sender lookup returned None and the decision was silently
/// dropped — the orchestrator never woke up. The new code path:
///   1. upsert_decision writes the decision durably,
///   2. notify-waiter best-effort wakes any live driver,
///   3. ensure_driver_running spawns a fresh driver if none is alive,
///   4. the new driver reconciles the gate_decisions row on its first
///      loop iteration and applies the recorded decision.
#[tokio::test]
async fn test_gate_decide_recovers_after_driver_death() {
    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_gate_recover_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let conn = crate::db::init_db(temp_dir.clone()).expect("init_db failed");
    let db = Arc::new(SqliteAdapter::new(conn).unwrap());
    let registry = Arc::new(AgentRegistry::new(vec![]));
    let notif = Arc::new(FakeNotif);
    let agent_exec = Arc::new(FakeAgentExec);
    let exec = Arc::new(FakeExec);
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
        artifacts,
        attachments,
        db.clone(), // attachment_json — SqliteAdapter implements both ports
        temp_dir.clone(),
        pricing,
    );

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;
    let gates: &dyn GateRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-recov"),
            name: "test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();

    features
        .add(Feature {
            id: FeatureId::from("f-recov"),
            project_id: ProjectId::from("p-recov"),
            workflow_id: Some(WorkflowId::from("w-recov")),
            title: "test feature".to_string(),
            status: "awaiting_gate".to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            created_at: now,
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
        })
        .unwrap();

    let se_id = StepExecutionId::from("se-recov");
    features
        .step_create(StepExecution {
            id: se_id.clone(),
            feature_id: FeatureId::from("f-recov"),
            step_id: StepId::from("step-1"),
            step_index: 0,
            step_kind: "gate".to_string(),
            status: "awaiting_gate".to_string(),
            cost_usd: Some(0.0),
            tokens: Some(0),
            wall_clock_secs: Some(0),
            artifact_path: None,
            artifact_paths: Vec::new(),
            error_message: None,
            iteration_count: 0,
            created_at: now,
            updated_at: now,
        })
        .unwrap();

    // Simulate the post-restart state: no live waiter, no live driver,
    // but a decided gate row (the user already pressed Approve).
    gates
        .upsert_decision(&se_id, "approve", Some("ship it"), now)
        .unwrap();

    // Pre-condition: the row is durably recorded.
    let recorded = gates
        .latest_for_step(&se_id)
        .unwrap()
        .expect("decided row should be present");
    assert_eq!(recorded.decision.as_deref(), Some("approve"));
    assert_eq!(recorded.feedback.as_deref(), Some("ship it"));

    // Now route through the presenter — exactly the path the Tauri
    // IPC command takes. It must (a) succeed without panicking on
    // missing waiter, (b) keep the DB row consistent, (c) attempt to
    // ensure_driver_running (no-op here since the step's workflow is
    // not wired into a real driver, but it must not error).
    executor
        .gate_decide("se-recov", "approve", Some("ship it"))
        .await
        .expect("gate_decide after driver death should succeed");

    let again = gates.latest_for_step(&se_id).unwrap().unwrap();
    assert_eq!(again.decision.as_deref(), Some("approve"));
    assert_eq!(again.feedback.as_deref(), Some("ship it"));

    // Idempotency: a re-delivery via the repo must not change the row.
    gates
        .upsert_decision(&se_id, "approve", Some("ship it"), now)
        .unwrap();
    let once_more = gates.latest_for_step(&se_id).unwrap().unwrap();
    assert_eq!(once_more.decision.as_deref(), Some("approve"));

    let _ = std::fs::remove_dir_all(temp_dir);
}
