//! Restart reconciliation against a feature a `demeteo-runner` still owns.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{Receiver, Sender},
    Arc, Mutex,
};

use super::harness::{FakeAgentExec, FakeNotif};
use crate::adapters::agent::registry::AgentRegistry;
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::adapters::step_executor::DagStepExecutor;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, StepId, WorkflowId};
use crate::domain::models::{Feature, StepExecution};
use crate::error::AppError;
use crate::paths;
use crate::ports::db::{FeatureRepository, GateRepository, ProjectRepository};
use crate::ports::step_executor::{GatePresenter, StepExecutor};

/// Strict runner-RPC fixture for the cleanup/reconciliation regression.
/// Every non-RPC operation is rejected, and each RPC call is retained so the
/// test can prove dismissal stopped reconciliation before shadow hydration.
struct RecordingRunnerRpc {
    calls: Mutex<Vec<(String, String)>>,
}

/// Pauses the first reconciliation list after it has returned the real rows.
/// This gives the regression a deterministic stale-snapshot interleaving.
struct ListPauseMirror {
    inner: Arc<dyn crate::ports::remote_run_mirror::RemoteRunMirrorPort>,
    listed: Sender<()>,
    resume: Mutex<Receiver<()>>,
    paused: AtomicBool,
}

impl crate::ports::remote_run_mirror::RemoteRunMirrorPort for ListPauseMirror {
    fn upsert_submitted(
        &self,
        machine_id: &str,
        run_id: &str,
        project_id: Option<&str>,
        feature_id: Option<&str>,
        title: &str,
        now: i64,
    ) -> Result<crate::ports::remote_run_mirror::RemoteRunMirror, String> {
        self.inner
            .upsert_submitted(machine_id, run_id, project_id, feature_id, title, now)
    }

    fn update_status(
        &self,
        machine_id: &str,
        run_id: &str,
        status: &str,
        error: Option<&str>,
        feature_id: Option<&str>,
        pr_url: Option<&str>,
        pushed_branch: Option<&str>,
        last_offset: i64,
        now: i64,
    ) -> Result<(), String> {
        self.inner.update_status(
            machine_id,
            run_id,
            status,
            error,
            feature_id,
            pr_url,
            pushed_branch,
            last_offset,
            now,
        )
    }

    fn mark_notified(&self, machine_id: &str, run_id: &str, status: &str) -> Result<(), String> {
        self.inner.mark_notified(machine_id, run_id, status)
    }

    fn delete_for_feature(&self, feature_id: &str) -> Result<(), String> {
        self.inner.delete_for_feature(feature_id)
    }

    fn get(
        &self,
        machine_id: &str,
        run_id: &str,
    ) -> Result<Option<crate::ports::remote_run_mirror::RemoteRunMirror>, String> {
        self.inner.get(machine_id, run_id)
    }

    fn list(&self) -> Result<Vec<crate::ports::remote_run_mirror::RemoteRunMirror>, String> {
        let rows = self.inner.list()?;
        if !self.paused.swap(true, Ordering::SeqCst) {
            self.listed
                .send(())
                .map_err(|_| "test did not wait for mirror list".to_string())?;
            self.resume
                .lock()
                .unwrap()
                .recv()
                .map_err(|_| "test did not resume reconciliation".to_string())?;
        }
        Ok(rows)
    }
}

impl RecordingRunnerRpc {
    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::ports::execution::ExecutionPort for RecordingRunnerRpc {
    async fn test_connection(&self, _: &str) -> Result<(), String> {
        Err("unexpected test_connection".to_string())
    }

    async fn run_command_with(
        &self,
        _: &str,
        _: &str,
        _: crate::ports::execution::ShellOptions,
    ) -> Result<String, String> {
        Err("unexpected run_command_with".to_string())
    }

    async fn read_file(&self, _: &str, _: &str) -> Result<String, String> {
        Err("unexpected read_file".to_string())
    }

    async fn write_file(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("unexpected write_file".to_string())
    }

    async fn write_file_bytes(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> {
        Err("unexpected write_file_bytes".to_string())
    }

    async fn get_metadata(
        &self,
        _: &str,
        _: &str,
    ) -> Result<crate::ports::execution::SftpEntry, String> {
        Err("unexpected get_metadata".to_string())
    }

    async fn list_dir(
        &self,
        _: &str,
        _: &str,
    ) -> Result<Vec<crate::ports::execution::SftpEntry>, String> {
        Err("unexpected list_dir".to_string())
    }

    async fn setup_worktree(&self, _: &str, _: &str, _: &str, _: &str) -> Result<(), String> {
        Err("unexpected setup_worktree".to_string())
    }

    async fn resolve_home(&self, _: &str) -> Result<String, String> {
        Err("unexpected resolve_home".to_string())
    }

    async fn resolve_user(&self, _: &str) -> Result<String, String> {
        Err("unexpected resolve_user".to_string())
    }

    async fn resolve_platform(&self, _: &str) -> Result<crate::domain::models::Platform, String> {
        Err("unexpected resolve_platform".to_string())
    }

    async fn control_rpc(
        &self,
        _: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let run_id = params
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "runner RPC omitted run_id".to_string())?
            .to_string();
        self.calls
            .lock()
            .unwrap()
            .push((method.to_string(), run_id.clone()));
        match (method, run_id.as_str()) {
            // This is deliberately configured although it must never be
            // consumed: cleanup removes this mirror before reconciliation.
            ("get_status", "r-dismissed") | ("get_status", "r-unrelated") => {
                Ok(serde_json::json!({ "status": "cancelled" }))
            }
            _ => Err(format!("unexpected runner RPC {method} for {run_id}")),
        }
    }

    fn spawn_interactive(
        &self,
        _: &str,
        _: &str,
        _: &[String],
        _: &str,
        _: &std::collections::HashMap<String, String>,
    ) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
        Err("unexpected spawn_interactive".to_string())
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cleanup_dismissal_blocks_a_stale_reconciliation_snapshot() {
    use crate::application::lifecycle::feature_cleanup;
    use crate::application::remote_runs::reconcile_all_runs;
    use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
    use crate::ports::remote_run_mirror::RemoteRunMirrorPort;

    let temp_dir = std::env::temp_dir().join(format!(
        "demeteo_test_cleanup_dismissal_{}",
        paths::now_ms()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: temp_dir.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(FakeNotif),
        tokio::runtime::Handle::current(),
    );
    let runner = Arc::new(RecordingRunnerRpc {
        calls: Mutex::new(Vec::new()),
    });
    ctx.exec = runner.clone();

    let now = paths::now_ms();
    let projects = ctx.projects.clone();
    let features = ctx.features.clone();
    let mirrors = ctx.remote_run_mirror.clone();
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-cleanup"),
            name: "cleanup fixture".to_string(),
            compute_type: "remote".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();
    let feature = |id: &str, status: &str| Feature {
        effort: None,
        id: FeatureId::from(id),
        project_id: ProjectId::from("p-cleanup"),
        workflow_id: None,
        workflow_version_id: None,
        title: id.to_string(),
        description: String::new(),
        status: status.to_string(),
        total_cost: 0.0,
        tokens: 0,
        duration: "0s".to_string(),
        agent_kind: None,
        model: None,
        mr_url: None,
        mr_state: Some("none".to_string()),
        pr_title: None,
        pr_body: None,
        created_at: now,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    };
    features.add(feature("f-dismissed", "running")).unwrap();

    mirrors
        .upsert_submitted(
            "m-1",
            "r-dismissed",
            Some("p-cleanup"),
            Some("f-dismissed"),
            "dismissed run",
            now,
        )
        .unwrap();
    mirrors
        .upsert_submitted("m-1", "r-unrelated", None, None, "unrelated run", now)
        .unwrap();

    let (listed_tx, listed_rx) = std::sync::mpsc::channel();
    let (resume_tx, resume_rx) = std::sync::mpsc::channel();
    let mirror: Arc<dyn RemoteRunMirrorPort> = Arc::new(ListPauseMirror {
        inner: ctx.remote_run_mirror.clone(),
        listed: listed_tx,
        resume: Mutex::new(resume_rx),
        paused: AtomicBool::new(false),
    });
    ctx.remote_run_mirror = mirror;
    let ctx = Arc::new(ctx);

    // The reconciliation task holds an in-memory copy of both rows, but is
    // paused before it can claim either one. Cleanup must be able to finish
    // in that window; when reconciliation resumes it must re-check the
    // mirror under the shared guard before any runner status/hydration work.
    let reconcile_ctx = ctx.clone();
    let reconciliation =
        tokio::spawn(async move { reconcile_all_runs(&reconcile_ctx, &|_| {}).await });
    listed_rx
        .recv()
        .expect("reconciliation must pause after listing mirrors");

    let result = feature_cleanup(&ctx, "f-dismissed".to_string(), None)
        .await
        .expect("archive cleanup succeeds");
    assert_eq!(result.action, "archived");
    assert_eq!(
        features
            .get(&FeatureId::from("f-dismissed"))
            .unwrap()
            .expect("cleanup keeps archived feature")
            .status,
        "archived"
    );

    resume_tx
        .send(())
        .expect("resume the stale reconciliation snapshot");
    let reconciled = reconciliation
        .await
        .expect("reconciliation task must complete")
        .unwrap();
    assert_eq!(
        reconciled
            .iter()
            .map(|row| row.run_id.as_str())
            .collect::<Vec<_>>(),
        ["r-unrelated"],
        "the dismissed run must never enter ordinary reconciliation"
    );
    assert_eq!(
        runner.calls(),
        [("get_status".to_string(), "r-unrelated".to_string())],
        "only the unrelated mirror may reach the runner; any hydration RPC is unexpected"
    );
    assert_eq!(
        features
            .get(&FeatureId::from("f-dismissed"))
            .unwrap()
            .expect("reconciliation must not recreate or overwrite the feature")
            .status,
        "archived"
    );

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
    let exec = Arc::new(ScriptedExec::new(&[]));
    let artifacts: Arc<dyn crate::ports::artifact_store::ArtifactStore> = Arc::new(
        crate::adapters::artifact_store::fs::FsArtifactStore::new(temp_dir.clone()),
    );
    let attachments: Arc<dyn crate::ports::attachment_store::AttachmentStore> =
        Arc::new(crate::adapters::attachment_store::fs::FsAttachmentStore::new(temp_dir.clone()));

    let sync_turns = Arc::new(crate::application::sync_turns::SyncTurns::default());
    let merge_executor: Arc<dyn crate::ports::merge::MergeExecutor> = {
        let git_ops =
            crate::adapters::worktree::git_ops::GitOpsHelper::new(db.clone(), exec.clone());
        Arc::new(crate::adapters::merge::SqliteMergeExecutor::new(
            db.clone(),
            db.clone(),
            db.clone(),
            sync_turns.clone(),
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
        db.clone(), // subtask_runs — SqliteAdapter implements the port
        db.clone(), // sequence_resume — SqliteAdapter implements the port
        artifacts,
        attachments,
        db.clone(), // attachment_json — SqliteAdapter implements both ports
        temp_dir.clone(),
        pricing,
        db.clone(), // remote-run mirror — SqliteAdapter implements the port
        sync_turns,
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
        effort: None,
        id: FeatureId::from(id.to_string()),
        project_id: ProjectId::from("p-1"),
        workflow_id: Some(WorkflowId::from("w-1")),
        workflow_version_id: None,
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
        pr_title: None,
        pr_body: None,
        created_at: now,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
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

    // ...and `gate_decide` itself must refuse the shadow rather than
    // upserting a decision no engine reads and returning `Ok` — the
    // driver-spawn refusal above is only logged, so a silent success here
    // is what the user experienced as "I clicked Approve and nothing
    // happened" on a detached run. The decision belongs on the runner.
    let err = GatePresenter::gate_decide(&*executor, "se-shadow", "approve", None)
        .await
        .expect_err("gate_decide must refuse a runner-owned shadow");
    assert!(
        matches!(&err, AppError::Validation { message } if message.contains("read-only shadow")),
        "expected the shadow refusal, got: {err:?}"
    );
    let gates: &dyn GateRepository = &*db;
    assert!(
        gates
            .pending_for_feature(&FeatureId::from("f-shadow"))
            .unwrap()
            .is_none_or(|g| g.decision.is_none()),
        "the refused gate_decide must not have written a local decision"
    );

    // Cancelling a shadow is the same story: the only cancel sender that
    // can stop the run lives on the runner, so signalling the (empty)
    // local map and reporting success is a "Stop" that does nothing.
    let err = StepExecutor::feature_cancel(&*executor, "f-shadow")
        .await
        .expect_err("feature_cancel must refuse a runner-owned shadow");
    assert!(
        err.contains("read-only shadow"),
        "expected the shadow refusal, got: {err}"
    );

    // Retry is the dangerous one: `replay_steps_from` calls
    // `start_execution_loop` directly, bypassing `ensure_driver_running`'s
    // guard, so without this refusal a retry would arm a second driver
    // against a run the runner still owns.
    features
        .step_update(
            &StepExecutionId::from("se-shadow"),
            &crate::ports::db::StepExecutionPatch {
                status: Some("failed".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
    let err = StepExecutor::step_retry(&*executor, "se-shadow", None, None, None)
        .await
        .expect_err("step_retry must refuse a runner-owned shadow");
    assert!(
        matches!(&err, AppError::Validation { message } if message.contains("read-only shadow")),
        "expected the shadow refusal, got: {err:?}"
    );
    assert!(
        !executor
            .driver_registry()
            .is_live(&FeatureId::from("f-shadow")),
        "the refused step_retry must not have armed a driver"
    );

    // `replay_from_step` reaches the same primitive by a different door and
    // skips `step_retry`'s status checks entirely, so it needs the guard in
    // `replay_steps_from` itself to be refused.
    let err = StepExecutor::replay_from_step(&*executor, "se-shadow", None, None, None)
        .await
        .expect_err("replay_from_step must refuse a runner-owned shadow");
    assert!(
        err.contains("read-only shadow"),
        "expected the shadow refusal, got: {err}"
    );
    assert!(
        !executor
            .driver_registry()
            .is_live(&FeatureId::from("f-shadow")),
        "the refused replay_from_step must not have armed a driver"
    );

    let _ = std::fs::remove_dir_all(temp_dir);
}
