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
use crate::error::AppError;
use crate::paths;
use crate::ports::agent_execution::{ActionError, AgentExecutionPort, CommandOutcome};
use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use crate::ports::execution::SftpEntry;
use crate::ports::execution::{ExecutionPort, InteractiveHandle};
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::{GatePresenter, StepExecutor};

struct FakeNotif;
impl NotificationPort for FakeNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// A `NotificationPort` that records every emitted event, so a test can
/// assert on the `BootstrapProgress` / `FeatureStatusChanged` sequence a
/// bootstrap produces.
#[derive(Default)]
struct CapturingNotif {
    events: std::sync::Mutex<Vec<DomainEvent>>,
}
impl NotificationPort for CapturingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
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
    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
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
    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Ok("fake".to_string())
    }
    async fn control_rpc(
        &self,
        _: &str,
        _: &str,
        _: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        Err("control_rpc not supported by this stub".to_string())
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
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
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
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
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
            description: String::new(),
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
            last_failure_fingerprint: None,
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
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
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
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
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
            description: String::new(),
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
            last_failure_fingerprint: None,
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
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
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

/// Helper: build a fully wired `DagStepExecutor` backed by an isolated
/// on-disk SQLite DB. Used by every guard test below — keeps the
/// boilerplate out of each test body. Returns `(executor, db, temp_dir)`
/// so callers can poke at the DB directly when needed.
async fn build_test_executor(
    label: &str,
) -> (DagStepExecutor, Arc<SqliteAdapter>, std::path::PathBuf) {
    build_test_executor_with_notif(label, Arc::new(FakeNotif)).await
}

async fn build_test_executor_with_notif(
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
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
    );
    (executor, db, temp_dir)
}

/// `step_retry` on a failed step whose predecessor is still in
/// `running` (or any non-terminal) status must be rejected with
/// `AppError::validation` naming the blocking step. This is the core
/// race-guard: a stale retry click must not unblock the executor while
/// an earlier step is still in flight.
#[tokio::test]
async fn test_step_retry_blocked_by_active_predecessor() {
    let (executor, db, temp_dir) = build_test_executor("retry_blocked").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-guard"),
            name: "guard-test".to_string(),
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
            id: FeatureId::from("f-guard"),
            project_id: ProjectId::from("p-guard"),
            workflow_id: Some(WorkflowId::from("w-guard")),
            title: "guard feature".to_string(),
            description: String::new(),
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

    // Three steps:
    //   index 0: completed        (terminal — not a blocker)
    //   index 1: running          (BLOCKER — must be named in the error)
    //   index 2: failed           (target for retry)
    for (idx, status) in [(0u32, "completed"), (1, "running"), (2, "failed")] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-guard-{idx}")),
                feature_id: FeatureId::from("f-guard"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: "agent".to_string(),
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    let err = executor
        .step_retry("se-guard-2", None, None)
        .await
        .expect_err("retry must be blocked by a running predecessor");
    match err {
        AppError::Validation { message } => {
            assert!(
                message.contains("step-1"),
                "blocking step id must be named in the message, got: {message}"
            );
            assert!(
                message.contains("running"),
                "blocking status must be named in the message, got: {message}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// `gate_decide` on an `awaiting_gate` step whose predecessor is still
/// in `running` must also be rejected with `AppError::validation`.
/// Same race surface, same guard, same message contract — but routed
/// through the `GatePresenter` trait instead of `StepExecutor`.
#[tokio::test]
async fn test_gate_decide_blocked_by_active_predecessor() {
    let (executor, db, temp_dir) = build_test_executor("gate_blocked").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;
    let gates: &dyn GateRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-gg"),
            name: "gate-guard-test".to_string(),
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
            id: FeatureId::from("f-gg"),
            project_id: ProjectId::from("p-gg"),
            workflow_id: Some(WorkflowId::from("w-gg")),
            title: "gate guard feature".to_string(),
            description: String::new(),
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

    // Predecessor still verifying (another non-terminal status the
    // guard must catch), gate step in awaiting_gate.
    for (idx, status) in [(0u32, "verifying"), (1, "awaiting_gate")] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-gg-{idx}")),
                feature_id: FeatureId::from("f-gg"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: if idx == 1 {
                    "gate".to_string()
                } else {
                    "agent".to_string()
                },
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    gates
        .create(GateDecision {
            id: GateDecisionId::from("gd-gg-1"),
            step_execution_id: StepExecutionId::from("se-gg-1"),
            decision: None,
            feedback: None,
            created_at: now,
        })
        .unwrap();

    let err = executor
        .gate_decide("se-gg-1", "approve", None)
        .await
        .expect_err("gate decide must be blocked by a verifying predecessor");
    match err {
        AppError::Validation { message } => {
            assert!(
                message.contains("step-0"),
                "blocking step id must be named in the message, got: {message}"
            );
            assert!(
                message.contains("verifying"),
                "blocking status must be named in the message, got: {message}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Once the blocking predecessor transitions to a terminal state, the
/// guard must let the retry proceed. This is the symmetry check: the
/// guard rejects when it should, accepts when it should.
#[tokio::test]
async fn test_step_retry_unblocks_when_predecessor_is_terminal() {
    let (executor, db, temp_dir) = build_test_executor("retry_unblocks").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-unb"),
            name: "unblock-test".to_string(),
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
            id: FeatureId::from("f-unb"),
            project_id: ProjectId::from("p-unb"),
            workflow_id: Some(WorkflowId::from("w-unb")),
            title: "unblock feature".to_string(),
            description: String::new(),
            status: "failed".to_string(),
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

    // All earlier steps are terminal (completed, skipped, failed).
    // Only failed/skipped/completed are non-blocking; the guard must
    // short-circuit to Ok and let `replay_steps_from` take over (which
    // here will fail downstream because there's no real driver / no
    // git repo, but the guard itself must not be the one failing).
    for (idx, status) in [(0u32, "completed"), (1, "skipped"), (2, "failed")] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-unb-{idx}")),
                feature_id: FeatureId::from("f-unb"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: "agent".to_string(),
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    // The guard passes; what comes after is replay_steps_from which
    // expects a real git worktree + project setup. The fake exec
    // returns empty / stub data, so the call will error downstream —
    // but it MUST NOT be an AppError::Validation with the "still"
    // phrase (that would mean the guard fired when it shouldn't).
    let result = executor.step_retry("se-unb-2", None, None).await;
    if let Err(AppError::Validation { ref message }) = result {
        panic!("guard fired despite all predecessors being terminal: {message}");
    }
    // Any other Err (e.g. driver spawn failure, missing workflow) is
    // acceptable for this test — we only care that the guard didn't
    // false-positive.

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The `assert_no_active_predecessors` helper itself: directly
/// unit-test the precondition scan without going through the
/// `step_retry` / `gate_decide` plumbing. Easier to assert the exact
/// message format and the precedence rule (lower `step_index`
/// wins when multiple predecessors are non-terminal).
#[tokio::test]
async fn test_assert_no_active_predecessors_helper() {
    let (executor, db, temp_dir) = build_test_executor("helper").await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-h"),
            name: "helper-test".to_string(),
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
            id: FeatureId::from("f-h"),
            project_id: ProjectId::from("p-h"),
            workflow_id: Some(WorkflowId::from("w-h")),
            title: "helper".to_string(),
            description: String::new(),
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

    // 5 steps: targets index 4. Preds at index 0 (done), 1 (done),
    // 2 (running — the EARLIEST non-terminal — must be reported),
    // 3 (awaiting_gate — also non-terminal but later in the scan).
    for (idx, status) in [
        (0u32, "completed"),
        (1, "failed"),
        (2, "running"),
        (3, "awaiting_gate"),
        (4, "failed"),
    ] {
        features
            .step_create(StepExecution {
                last_failure_fingerprint: None,
                id: StepExecutionId::from(format!("se-h-{idx}")),
                feature_id: FeatureId::from("f-h"),
                step_id: StepId::from(format!("step-{idx}")),
                step_index: idx,
                step_kind: "agent".to_string(),
                status: status.to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    let target = features
        .step_get(&StepExecutionId::from("se-h-4".to_string()))
        .unwrap()
        .unwrap();

    let err = executor
        .assert_no_active_predecessors(&target, "retrying this step")
        .expect_err("must report the earliest non-terminal predecessor");
    match err {
        AppError::Validation { message } => {
            // step-2 has the lowest step_index among non-terminal
            // predecessors, so it must be the one named.
            assert!(
                message.contains("step-2"),
                "expected step-2 to be named, got: {message}"
            );
            assert!(
                !message.contains("step-3"),
                "later non-terminal pred must not be picked, got: {message}"
            );
            assert!(
                message.contains("retrying this step"),
                "intent phrase must be threaded through, got: {message}"
            );
        }
        other => panic!("expected AppError::Validation, got: {other:?}"),
    }

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Restart reconciliation must leave runner-owned shadow features alone
/// (C4.2): a feature whose id appears in the remote-run mirror is a
/// read-only copy of something a `demeteo-runner` is still driving on
/// another machine. Before the `runner_owned` skip-set existed, an app
/// restart while a detached run was live would mark the shadow's steps
/// `interrupted` and re-arm a local driver against it — two engines
/// driving one feature.
#[tokio::test]
async fn watchdog_and_resume_skip_runner_owned_shadows() {
    use crate::ports::remote_run_mirror::RemoteRunMirrorPort;

    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_watchdog_shadow_{}",
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
    let executor = Arc::new(DagStepExecutor::new(
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
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
    ));

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    let features: &dyn FeatureRepository = &*db;

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

    // Two features left "running" across a restart: `f-shadow` is a
    // mirror-listed shadow of a live detached run; `f-local` is a real
    // local feature whose process died.
    let mk_feature = |id: &str| Feature {
        id: FeatureId::from(id.to_string()),
        project_id: ProjectId::from("p-1"),
        workflow_id: Some(WorkflowId::from("w-1")),
        title: id.to_string(),
        description: String::new(),
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
    };
    let mk_step = |se: &str, f: &str| StepExecution {
        last_failure_fingerprint: None,
        id: StepExecutionId::from(se.to_string()),
        feature_id: FeatureId::from(f.to_string()),
        step_id: StepId::from("step-1"),
        step_index: 0,
        step_kind: "agent".to_string(),
        status: "running".to_string(),
        cost_usd: Some(0.0),
        tokens: Some(0),
        wall_clock_secs: Some(0),
        artifact_path: None,
        artifact_paths: Vec::new(),
        error_message: None,
        iteration_count: 0,
        cache_read_input_tokens: None,
        cache_creation_input_tokens: None,
        created_at: now,
        updated_at: now,
    };
    features.add(mk_feature("f-shadow")).unwrap();
    features
        .step_create(mk_step("se-shadow", "f-shadow"))
        .unwrap();
    features.add(mk_feature("f-local")).unwrap();
    features
        .step_create(mk_step("se-local", "f-local"))
        .unwrap();

    // The mirror row is the runner-owned marker (C4.2) — the executor
    // reads it through its own mirror port, exactly like production.
    let mirror: &dyn RemoteRunMirrorPort = &*db;
    mirror
        .upsert_submitted("m1", "r1", Some("p-1"), Some("f-shadow"), "f-shadow", now)
        .unwrap();

    executor.startup_watchdog();

    // The shadow is untouched — still mirroring whatever the runner says.
    let shadow = features.get(&FeatureId::from("f-shadow")).unwrap().unwrap();
    assert_eq!(shadow.status, "running");
    let shadow_step = features
        .step_get(&StepExecutionId::from("se-shadow"))
        .unwrap()
        .unwrap();
    assert_eq!(shadow_step.status, "running");

    // The genuinely-local feature got the normal restart treatment.
    let local = features.get(&FeatureId::from("f-local")).unwrap().unwrap();
    assert_eq!(local.status, "awaiting_gate");
    let local_step = features
        .step_get(&StepExecutionId::from("se-local"))
        .unwrap()
        .unwrap();
    assert_eq!(local_step.status, "interrupted");

    // Resume must not arm a local driver against the shadow, even if the
    // runner has it parked on a gate (the state resume normally re-arms).
    features
        .update(
            &FeatureId::from("f-shadow"),
            &crate::ports::db::FeaturePatch {
                status: Some("awaiting_gate".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    executor.clone().resume_interrupted_features().await;
    assert!(
        !executor
            .driver_registry()
            .is_live(&FeatureId::from("f-shadow")),
        "a runner-owned shadow must never get a local driver"
    );

    // The guard lives in `ensure_driver_running` itself, so every
    // recovery path is covered — including `gate_decide`'s self-healing
    // respawn, which a shadow step parked in `awaiting_gate` can reach.
    let err = executor
        .ensure_driver_running("f-shadow")
        .await
        .expect_err("ensure_driver_running must refuse a runner-owned shadow");
    assert!(
        err.contains("read-only shadow"),
        "expected the shadow refusal, got: {err}"
    );
    assert!(
        !executor
            .driver_registry()
            .is_live(&FeatureId::from("f-shadow")),
        "the refused ensure_driver_running must not have armed a driver"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}

/// `feature_start` inserts the eager row as `bootstrapping` and returns
/// immediately, then runs the bootstrap on a spawned tail that streams
/// `BootstrapProgress` events. A bootstrap that fails (here: a project with no
/// repository, so `resolve_execution_context` bails at the repo check) must
/// emit a `preparing` "failed" phase and drive the feature to `failed` via a
/// `FeatureStatusChanged` — rather than the pre-refactor behavior of returning
/// an error straight from `feature_start`.
#[tokio::test]
async fn test_feature_start_bootstrap_failure_emits_events_and_fails() {
    let notif = Arc::new(CapturingNotif::default());
    let (executor, db, temp_dir) = build_test_executor_with_notif("boot_fail", notif.clone()).await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-boot"),
            name: "boot-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();
    // Deliberately no repository for the project → the bootstrap tail's
    // `resolve_execution_context` fails at the repo check.

    let feature = executor
        .feature_start(
            None,
            "p-boot",
            "wf-x",
            "Boot Feature",
            "a description",
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start returns the eager row, not an error");

    // The returned row is the eager, pre-bootstrap snapshot.
    assert_eq!(
        feature.status, "bootstrapping",
        "feature_start returns immediately with a bootstrapping row"
    );

    // Wait for the spawned tail to reconcile the feature to a terminal state.
    let features: &dyn FeatureRepository = &*db;
    let fid = FeatureId::from(feature.id.0.clone());
    let mut final_status = String::new();
    for _ in 0..200 {
        if let Ok(Some(f)) = features.get(&fid) {
            if f.status == "failed" {
                final_status = f.status;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        final_status, "failed",
        "the failed bootstrap tail marks the feature failed"
    );

    let events = notif.events.lock().unwrap();
    let saw_preparing_failed = events.iter().any(|e| {
        matches!(
            e,
            DomainEvent::BootstrapProgress { phase, status, .. }
                if phase == "preparing" && status == "failed"
        )
    });
    assert!(
        saw_preparing_failed,
        "expected a BootstrapProgress{{preparing, failed}} event; got: {:?}",
        *events
    );
    let saw_status_failed = events.iter().any(|e| {
        matches!(
            e,
            DomainEvent::FeatureStatusChanged { status, .. } if status == "failed"
        )
    });
    assert!(
        saw_status_failed,
        "expected a FeatureStatusChanged(failed) event"
    );

    drop(events);
    let _ = std::fs::remove_dir_all(temp_dir);
}
