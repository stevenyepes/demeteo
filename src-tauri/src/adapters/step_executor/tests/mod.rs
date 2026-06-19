use std::sync::Arc;

use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::DagStepExecutor;
use crate::domain::action::AgentAction;
use crate::domain::ids::{FeatureId, GateDecisionId, ProjectId, StepExecutionId, StepId, WorkflowId};
use crate::domain::intercept::ExecutionResult;
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::sftp::SftpEntry;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::{GatePresenter, StepExecutor};
use crate::paths;

struct FakeNotif;
impl NotificationPort for FakeNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

struct FakeAgentExec;
impl AgentExecutionPort for FakeAgentExec {
    fn submit(&self, _: &str, _: &str, _: AgentAction) -> Result<CommandOutcome, String> {
        Ok(CommandOutcome::Executed {
            output: ExecutionResult::Bash {
                output: String::new(),
            },
        })
    }
    fn submit_agent(
        &self,
        _: &str,
        _: &str,
        _: AgentAction,
        _: Option<String>,
    ) -> Result<CommandOutcome, ActionError> {
        Err(ActionError::internal("stub"))
    }
    fn approve(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn reject(&self, _: &str, _: String) -> Result<(), String> {
        Ok(())
    }
    fn register_result_responder(
        &self,
        _: &str,
        _: tokio::sync::oneshot::Sender<Result<ExecutionResult, String>>,
    ) -> Result<(), String> {
        Ok(())
    }
}

struct FakeExec;
impl ExecutionPort for FakeExec {
    fn test_connection(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn run_command(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(String::new())
    }
    fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn get_metadata(&self, _: &str, path: &str) -> Result<SftpEntry, String> {
        Ok(SftpEntry {
            name: path.into(),
            path: path.into(),
            is_dir: false,
            size: 0,
            modified: 0,
        })
    }
    fn list_dir(&self, _: &str, _: &str) -> Result<Vec<SftpEntry>, String> {
        Ok(vec![])
    }
    fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn resolve_home(&self, _: &str) -> Result<String, String> {
        Ok("/tmp".to_string())
    }
    fn spawn_interactive(&self, _: &str, _: &str, _: &[String], _: &str, _: &std::collections::HashMap<String, String>) -> Result<Box<dyn InteractiveHandle>, String> {
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
    let artifacts: Arc<dyn crate::ports::artifact_store::ArtifactStore> =
        Arc::new(crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()));

    let executor = DagStepExecutor::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        registry,
        notif,
        agent_exec,
        exec,
        artifacts,
        temp_dir.clone(),
    );

    let cancel_res = executor.feature_cancel("f-nonexistent");
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
    let artifacts: Arc<dyn crate::ports::artifact_store::ArtifactStore> =
        Arc::new(crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()));

    let executor = DagStepExecutor::new(
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        db.clone(),
        registry,
        notif,
        agent_exec,
        exec,
        artifacts,
        temp_dir.clone(),
    );

    let (tx, rx) = tokio::sync::oneshot::channel::<GateDecision>();
    executor
        .gate_senders
        .lock()
        .unwrap()
        .insert("se-1".to_string(), tx);

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
            duration: "0s".to_string(),
            agent_kind: None,
            model: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            created_at: now,
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

    let decide_res = executor.gate_decide("se-1", "approve", Some("looks good"));
    assert!(decide_res.is_ok());

    let decision = rx.await.unwrap();
    assert_eq!(decision.decision.as_deref(), Some("approve"));
    assert_eq!(decision.feedback.as_deref(), Some("looks good"));

    let _ = std::fs::remove_dir_all(temp_dir);
}
