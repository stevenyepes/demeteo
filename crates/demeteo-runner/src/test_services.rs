// Runner services over a real database, for the tests of what stops a run.
// Only the executor is a double: it records every `feature_cancel` and errors
// on any other call, so "the executor was not asked" is read from what it
// recorded rather than from a default answer. `Rig::with_engine` keeps the
// real one instead, for what a cancel leaves on the feature row.

use crate::away_notify::NoopAwayNotifier;
use crate::credentials::{CredentialStore, RunnerCredentialSource};
use crate::services::RunnerServices;
use async_trait::async_trait;
use demeteo_core::adapters::database::SqliteAdapter;
use demeteo_core::adapters::hub::sealed_store::SealedSecretStore;
use demeteo_core::adapters::notification_noop::NoopNotificationAdapter;
use demeteo_core::composition::{build_core_context_with_hub_store, CoreConfig, ExecutionMode};
use demeteo_core::domain::feature_origin::FeatureOrigin;
use demeteo_core::domain::hub_pairing::sealed::SEALED_BACKEND;
use demeteo_core::domain::ids::{FeatureId, ProjectId};
use demeteo_core::domain::models::{
    EffortLevel, Feature, FeatureDivergence, FeatureDrift, Project, StepExecution,
};
use demeteo_core::domain::step_assignment::StepAssignment;
use demeteo_core::domain::sync_resolver::SyncResolverChoice;
use demeteo_core::domain::upstream_feature::DivergenceReconcile;
use demeteo_core::error::AppError;
use demeteo_core::ports::runner_run::RunnerRun;
use demeteo_core::ports::step_executor::{
    FeatureLaunch, StepExecutor, SyncOutcomeView, SyncResolverView,
};
use demeteo_core::ports::sync_session::SyncSessionView;
use demeteo_core::test_dir::TestDir;
use std::sync::{Arc, Mutex};

pub(crate) struct Rig {
    _dir: TestDir,
    pub svc: Arc<RunnerServices>,
    pub executor: Arc<CancelSpy>,
}

impl Rig {
    pub fn new(tag: &str) -> Self {
        Self::build(tag, true)
    }

    /// The composed `DagStepExecutor`, with no driver running for any feature.
    /// `executor` is then never called and records nothing.
    pub fn with_engine(tag: &str) -> Self {
        Self::build(tag, false)
    }

    fn build(tag: &str, spy: bool) -> Self {
        let dir = TestDir::new(tag);
        let db = SqliteAdapter::new(demeteo_core::db::init_db(dir.path().to_path_buf()).unwrap())
            .unwrap();
        let creds = Arc::new(CredentialStore::new());
        let (mut ctx, _writer, _binding) = build_core_context_with_hub_store(
            CoreConfig {
                app_data_dir: dir.path().to_path_buf(),
                execution_mode: ExecutionMode::LocalOnly,
            },
            Arc::new(NoopNotificationAdapter),
            tokio::runtime::Handle::current(),
            Arc::new(RunnerCredentialSource::new(creds.clone(), dir.path())),
            Arc::new(SealedSecretStore::new(
                Arc::new(db),
                Err("no key in a test"),
                SEALED_BACKEND,
            )),
        );
        let executor = Arc::new(CancelSpy::default());
        if spy {
            ctx.executor = executor.clone();
        }
        Self {
            svc: Arc::new(RunnerServices {
                ctx: Arc::new(ctx),
                creds,
                askpass_path: dir.path().join("askpass"),
                away_notifier: Arc::new(NoopAwayNotifier),
                drivers: Arc::default(),
            }),
            executor,
            _dir: dir,
        }
    }

    /// A run of `feature_id` owned by `client_id`, holding a credential.
    pub fn seed_run(&self, run_id: &str, feature_id: &str, client_id: &str, status: &str) {
        let runs = &self.svc.ctx.runner_runs;
        runs.get_or_create(run_id, "{}", client_id, 1).unwrap();
        runs.update_status(run_id, status, Some("p-1"), Some(feature_id), None, None, 2)
            .unwrap();
        self.svc.creds.insert(run_id, format!("pat-of-{run_id}"));
    }

    /// A feature row at `status`, under a project of its own.
    pub fn seed_feature(&self, feature_id: &str, status: &str) {
        let ctx = &self.svc.ctx;
        ctx.projects
            .add(Project {
                id: ProjectId::from("p-1"),
                name: "p-1".to_string(),
                compute_type: "local".to_string(),
                remote_host: None,
                status: "idle".to_string(),
                nodes: 0,
                spend: 0.0,
                tokens: 0,
                created_at: 1,
            })
            .unwrap();
        ctx.features
            .add(Feature {
                id: FeatureId::from(feature_id.to_string()),
                project_id: ProjectId::from("p-1"),
                workflow_id: None,
                workflow_version_id: None,
                title: feature_id.to_string(),
                description: String::new(),
                status: status.to_string(),
                total_cost: 0.0,
                duration: "0s".to_string(),
                tokens: 0,
                created_at: 1,
                agent_kind: None,
                model: None,
                effort: None,
                mr_url: None,
                mr_state: Some("none".to_string()),
                pr_title: None,
                pr_body: None,
                commit_artifacts: None,
                loop_iterations: None,
                max_budget_usd: None,
                step_overrides: Vec::new(),
                attachments: Vec::new(),
                harness_baseline: None,
                origin: FeatureOrigin::DefaultBranch,
                diff_base_branch: None,
                resolved_branch: None,
                launched_by: Default::default(),
            })
            .unwrap();
    }

    pub fn feature_status(&self, feature_id: &str) -> String {
        self.svc
            .ctx
            .features
            .get(&FeatureId::from(feature_id.to_string()))
            .unwrap()
            .expect("feature")
            .status
    }

    pub fn run(&self, run_id: &str) -> RunnerRun {
        self.svc.ctx.runner_runs.get(run_id).unwrap().expect("run")
    }

    pub fn event_kinds(&self, run_id: &str) -> Vec<String> {
        self.svc
            .ctx
            .run_events
            .list_since(run_id, 0)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct CancelSpy {
    cancels: Mutex<Vec<String>>,
}

impl CancelSpy {
    pub fn cancels(&self) -> Vec<String> {
        self.cancels.lock().unwrap().clone()
    }
}

fn unscripted<T>(call: &str) -> Result<T, String> {
    Err(format!("unscripted StepExecutor call: {call}"))
}

#[async_trait]
impl StepExecutor for CancelSpy {
    async fn feature_start(&self, _: FeatureLaunch) -> Result<Feature, String> {
        unscripted("feature_start")
    }
    async fn feature_pause(&self, _: &str) -> Result<(), String> {
        unscripted("feature_pause")
    }
    async fn feature_resume(&self, _: &str) -> Result<(), String> {
        unscripted("feature_resume")
    }
    async fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
        self.cancels.lock().unwrap().push(feature_id.to_owned());
        Ok(())
    }
    async fn step_get(&self, _: &str) -> Result<StepExecution, String> {
        unscripted("step_get")
    }
    async fn step_retry(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), AppError> {
        Ok(unscripted("step_retry")?)
    }
    async fn step_set_assignment(&self, _: &str, _: StepAssignment) -> Result<(), AppError> {
        Ok(unscripted("step_set_assignment")?)
    }
    async fn replay_from_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), String> {
        unscripted("replay_from_step")
    }
    async fn step_list_for_run(&self, _: &str) -> Result<Vec<StepExecution>, String> {
        unscripted("step_list_for_run")
    }
    async fn feature_sync(&self, _: &str) -> Result<SyncOutcomeView, String> {
        unscripted("feature_sync")
    }
    async fn feature_drift(&self, _: &str, _: bool) -> Result<FeatureDrift, String> {
        unscripted("feature_drift")
    }
    async fn feature_reconcile(
        &self,
        _: &str,
        _: DivergenceReconcile,
    ) -> Result<Option<SyncSessionView>, String> {
        unscripted("feature_reconcile")
    }
    async fn feature_divergence(&self, _: &str) -> Result<Option<FeatureDivergence>, String> {
        unscripted("feature_divergence")
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _: &str,
        _: &SyncResolverChoice,
    ) -> Result<SyncOutcomeView, String> {
        unscripted("feature_resolve_sync_conflicts")
    }
    async fn feature_continue_sync(&self, _: &str) -> Result<SyncOutcomeView, String> {
        unscripted("feature_continue_sync")
    }
    async fn feature_sync_resolver(&self, _: &str) -> Result<SyncResolverView, String> {
        unscripted("feature_sync_resolver")
    }
}
