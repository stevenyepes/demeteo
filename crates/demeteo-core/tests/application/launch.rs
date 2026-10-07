// Tests for `src/application/launch.rs` (mirrored-tests convention). `super`
// resolves to that module.
//
// Both doubles refuse what they were not scripted for, so "the executor was
// not called" and "no RPC was issued" are read from what they recorded rather
// than from a default answer. `tests/application/tickets/launch.rs` drives
// tickets through the same `harness`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{
    FeatureId, MachineId, ProjectId, ProviderId, RepositoryId, WorkflowId, WorkflowVersionId,
};
use crate::domain::models::{
    EffortLevel, Machine, Platform, Project, ProviderInstance, Repository, Workflow,
    WorkflowVersion,
};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};
use crate::ports::remote_run_mirror::{RemoteRunMirror, RemoteRunMirrorPort};
use crate::ports::step_executor::StepExecutor;

pub(crate) const APP_VERSION: &str = "1.2.0-31";

/// Records the one call a launch may make; every other `StepExecutor` method
/// panics.
pub(crate) struct SpyExecutor {
    captured: Mutex<Vec<FeatureLaunch>>,
}

impl SpyExecutor {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            captured: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl StepExecutor for SpyExecutor {
    async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String> {
        let feature = Feature {
            id: FeatureId::from("f-1".to_string()),
            project_id: ProjectId::from(launch.project_id.clone()),
            workflow_id: None,
            workflow_version_id: None,
            title: launch.title.clone(),
            description: launch.description.clone(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: String::new(),
            tokens: 0,
            created_at: 0,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: None,
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
        };
        self.captured
            .lock()
            .expect("lock is not poisoned")
            .push(launch);
        Ok(feature)
    }

    async fn feature_pause(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_resume(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_cancel(&self, _: &str) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_get(&self, _: &str) -> Result<crate::domain::models::StepExecution, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_retry(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), crate::error::AppError> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_set_assignment(
        &self,
        _: &str,
        _: crate::domain::step_assignment::StepAssignment,
    ) -> Result<(), crate::error::AppError> {
        panic!("unexpected StepExecutor call")
    }
    async fn replay_from_step(
        &self,
        _: &str,
        _: Option<&str>,
        _: Option<&str>,
        _: Option<EffortLevel>,
    ) -> Result<(), String> {
        panic!("unexpected StepExecutor call")
    }
    async fn step_list_for_run(
        &self,
        _: &str,
    ) -> Result<Vec<crate::domain::models::StepExecution>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_sync(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_drift(
        &self,
        _: &str,
        _: bool,
    ) -> Result<crate::domain::models::FeatureDrift, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_reconcile(
        &self,
        _: &str,
        _: crate::domain::upstream_feature::DivergenceReconcile,
    ) -> Result<Option<crate::ports::sync_session::SyncSessionView>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_divergence(
        &self,
        _: &str,
    ) -> Result<Option<crate::domain::models::FeatureDivergence>, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _: &str,
        _: &crate::domain::sync_resolver::SyncResolverChoice,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_continue_sync(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncOutcomeView, String> {
        panic!("unexpected StepExecutor call")
    }
    async fn feature_sync_resolver(
        &self,
        _: &str,
    ) -> Result<crate::ports::step_executor::SyncResolverView, String> {
        panic!("unexpected StepExecutor call")
    }
}

impl SpyExecutor {
    pub(crate) fn launched(&self) -> Option<FeatureLaunch> {
        self.captured
            .lock()
            .expect("lock is not poisoned")
            .last()
            .cloned()
    }

    pub(crate) fn launch_count(&self) -> usize {
        self.captured.lock().expect("lock is not poisoned").len()
    }
}

/// How the runner answers the compatibility probe, and whether it then takes
/// the submit and the PAT. Every call it was not scripted for is recorded and
/// answered `Err`.
pub(crate) struct RunnerAt {
    /// `None`: `health` errors, so the probe falls back to `--version`.
    pub(crate) build_version: Option<&'static str>,
    pub(crate) home_resolves: bool,
    /// What the installed binary prints for `--version`; empty reads as not
    /// installed.
    pub(crate) binary_version: &'static str,
    pub(crate) accepts_submit: bool,
    pub(crate) accepts_credentials: bool,
    /// Answers `retry_step`, recorded with the run and step it named.
    pub(crate) accepts_retry: bool,
    pub(crate) calls: Mutex<Vec<String>>,
}

impl RunnerAt {
    pub(crate) fn reporting(build_version: &'static str) -> Self {
        Self {
            build_version: Some(build_version),
            home_resolves: true,
            binary_version: "",
            accepts_submit: false,
            accepts_credentials: false,
            accepts_retry: false,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn silent(home_resolves: bool) -> Self {
        Self {
            build_version: None,
            home_resolves,
            ..Self::reporting(APP_VERSION)
        }
    }

    pub(crate) fn accepting() -> Self {
        Self {
            accepts_submit: true,
            accepts_credentials: true,
            ..Self::reporting(APP_VERSION)
        }
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    pub(crate) fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn unscripted<T>(&self, call: &str) -> Result<T, String> {
        self.record(call.to_string());
        Err(format!("unscripted {call}"))
    }
}

#[async_trait]
impl ExecutionPort for RunnerAt {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        self.unscripted("test_connection")
    }
    async fn run_command(&self, _machine_id: &str, cmd: &str) -> Result<String, String> {
        self.record(format!("run_command {cmd}"));
        if cmd.contains("--version") {
            Ok(self.binary_version.to_string())
        } else {
            Err(format!("unscripted run_command {cmd}"))
        }
    }
    async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
        self.unscripted("read_file")
    }
    async fn write_file(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &str,
    ) -> Result<(), String> {
        self.unscripted("write_file")
    }
    async fn write_file_bytes(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &[u8],
    ) -> Result<(), String> {
        self.unscripted("write_file_bytes")
    }
    async fn get_metadata(&self, _machine_id: &str, _path: &str) -> Result<SftpEntry, String> {
        self.unscripted("get_metadata")
    }
    async fn list_dir(&self, _machine_id: &str, _path: &str) -> Result<Vec<SftpEntry>, String> {
        self.unscripted("list_dir")
    }
    async fn setup_worktree(
        &self,
        _machine_id: &str,
        _repo_path: &str,
        _branch: &str,
        _sandbox_path: &str,
    ) -> Result<(), String> {
        self.unscripted("setup_worktree")
    }
    async fn resolve_home(&self, _machine_id: &str) -> Result<String, String> {
        if self.home_resolves {
            self.record("resolve_home".to_string());
            Ok("/home/runner".to_string())
        } else {
            self.unscripted("resolve_home")
        }
    }
    async fn resolve_platform(&self, _machine_id: &str) -> Result<Platform, String> {
        self.unscripted("resolve_platform")
    }
    async fn resolve_user(&self, _machine_id: &str) -> Result<String, String> {
        self.unscripted("resolve_user")
    }
    async fn control_rpc(
        &self,
        _machine_id: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        if method == "retry_step" && self.accepts_retry {
            self.record(format!(
                "rpc retry_step {} {} {}",
                params["run_id"].as_str().unwrap_or_default(),
                params["step_execution_id"].as_str().unwrap_or_default(),
                params["mode"].as_str().unwrap_or_default(),
            ));
            return Ok(serde_json::json!({ "status": "running" }));
        }
        self.record(format!("rpc {method}"));
        match (method, self.build_version) {
            ("health", Some(version)) => Ok(serde_json::json!({ "build_version": version })),
            ("submit_run", _) if self.accepts_submit => {
                Ok(serde_json::json!({ "status": "pending" }))
            }
            ("inject_credentials", _) if self.accepts_credentials => Ok(serde_json::json!({})),
            _ => Err(format!("unscripted rpc {method}")),
        }
    }
    fn spawn_interactive(
        &self,
        _machine_id: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &HashMap<String, String>,
    ) -> Result<Box<dyn InteractiveHandle>, String> {
        self.unscripted("spawn_interactive")
    }
}

/// The mirror write a [`MirrorFailingOn`] refuses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MirrorWrite {
    Submitted,
    Status,
}

impl MirrorWrite {
    pub(crate) fn method(self) -> &'static str {
        match self {
            Self::Submitted => "upsert_submitted",
            Self::Status => "update_status",
        }
    }
}

/// The context's own mirror, except that one write errors — the local
/// SQLite failure that can land after the runner has already accepted a run.
pub(crate) struct MirrorFailingOn {
    inner: Arc<dyn RemoteRunMirrorPort>,
    failing: MirrorWrite,
}

impl MirrorFailingOn {
    pub(crate) fn wrap(
        inner: Arc<dyn RemoteRunMirrorPort>,
        failing: MirrorWrite,
    ) -> Arc<dyn RemoteRunMirrorPort> {
        Arc::new(Self { inner, failing })
    }

    fn refuse(&self, write: MirrorWrite) -> Result<(), String> {
        if self.failing == write {
            Err(format!("database is locked during {}", write.method()))
        } else {
            Ok(())
        }
    }
}

impl RemoteRunMirrorPort for MirrorFailingOn {
    fn upsert_submitted(
        &self,
        machine_id: &str,
        run_id: &str,
        project_id: Option<&str>,
        feature_id: Option<&str>,
        title: &str,
        now: i64,
    ) -> Result<RemoteRunMirror, String> {
        self.refuse(MirrorWrite::Submitted)?;
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
        self.refuse(MirrorWrite::Status)?;
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
    fn get(&self, machine_id: &str, run_id: &str) -> Result<Option<RemoteRunMirror>, String> {
        self.inner.get(machine_id, run_id)
    }
    fn list(&self) -> Result<Vec<RemoteRunMirror>, String> {
        self.inner.list()
    }
    fn list_for_features(&self, feature_ids: &[&str]) -> Result<Vec<RemoteRunMirror>, String> {
        self.inner.list_for_features(feature_ids)
    }
}

pub(crate) fn machine(id: &str, auth_type: &str) -> Machine {
    Machine {
        id: MachineId::from(id),
        name: id.to_string(),
        host: "runner.example".to_string(),
        port: 22,
        username: "dev".to_string(),
        auth_type: auth_type.to_string(),
        key_path: None,
        agents: None,
        auto_approved_rules: None,
        use_login_shell: None,
        setup_commands: None,
        notify_webhook_url: None,
    }
}

pub(crate) struct Harness {
    pub(crate) ctx: AppContext,
    pub(crate) exec: Arc<RunnerAt>,
    pub(crate) spy: Arc<SpyExecutor>,
    dir: std::path::PathBuf,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A context in which a detached submit to `runner-1` would get all the way
/// to the RPC: the machine, this app's version, the project with its
/// repository and provider, the workflow, and a PAT under a provider id no
/// other test uses.
pub(crate) fn harness(exec: RunnerAt) -> Harness {
    let dir = crate::support::test_dir::scratch("demeteo-launch-run");
    let exec = Arc::new(exec);
    let spy = SpyExecutor::new();
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    ctx.exec = exec.clone();
    ctx.executor = spy.clone();
    ctx.app_version.set(APP_VERSION.to_string());
    ctx.machines.add(machine("runner-1", "key")).unwrap();
    ctx.projects
        .add(Project {
            id: ProjectId::from("p-1"),
            name: "launched".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 0,
        })
        .unwrap();
    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from("prov-launch-run"),
            kind: "github".to_string(),
            host: "github.com".to_string(),
            username: "dev".to_string(),
            avatar_url: String::new(),
            created_at: 0,
        })
        .unwrap();
    ctx.projects
        .add_repository(Repository {
            id: RepositoryId::from("repo-1"),
            project_id: ProjectId::from("p-1"),
            provider_id: ProviderId::from("prov-launch-run"),
            repo_path: "org/repo".to_string(),
        })
        .unwrap();
    crate::credential_cache::set("prov-launch-run", "pat-launch-run");
    ctx.workflows
        .create(Workflow {
            id: WorkflowId::from("w-1"),
            name: "W".to_string(),
            description: String::new(),
            is_starter: false,
            created_at: 0,
            updated_at: 0,
            schedule: None,
        })
        .unwrap();
    ctx.workflows
        .save_version(WorkflowVersion {
            id: WorkflowVersionId::from("wv-1"),
            workflow_id: WorkflowId::from("w-1"),
            version: 1,
            steps_json: "[]".to_string(),
            definition_json: None,
            created_at: 0,
            note: None,
        })
        .unwrap();
    Harness {
        ctx,
        exec,
        spy,
        dir,
    }
}

fn feature_launch() -> FeatureLaunch {
    FeatureLaunch {
        feature_id: Some("f-chosen".to_string()),
        project_id: "p-1".to_string(),
        workflow_id: "w-1".to_string(),
        title: "Ship it".to_string(),
        description: "Launched through launch_run".to_string(),
        agent_kind: Some("claude-code".to_string()),
        model: Some("opus".to_string()),
        effort: Some(EffortLevel::High),
        loop_iterations: Some(2),
        origin: FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        },
        diff_base_branch: Some("release/2.0".to_string()),
        ..FeatureLaunch::default()
    }
}

fn detached(machine_id: &str) -> RunPlacement {
    RunPlacement::Detached {
        machine_id: MachineId::from(machine_id),
    }
}

fn request(placement: RunPlacement, detached: DetachedOptions) -> LaunchRequest {
    LaunchRequest {
        launch: feature_launch(),
        placement,
        detached,
    }
}

fn feature_ids(ctx: &AppContext) -> Vec<String> {
    ctx.features
        .get_all_for_project(&ProjectId::from("p-1"))
        .unwrap()
        .into_iter()
        .map(|feature| feature.id.0)
        .collect()
}

// --- AC1 ------------------------------------------------------------------

#[tokio::test]
async fn a_local_launch_hands_the_executor_the_launch_unchanged() {
    let h = harness(RunnerAt::reporting(APP_VERSION));

    let launched = launch_run(
        &h.ctx,
        request(RunPlacement::Local, DetachedOptions::default()),
    )
    .await
    .expect("a local launch starts");

    let received = h.spy.launched().expect("feature_start was called");
    assert_eq!(format!("{received:?}"), format!("{:?}", feature_launch()));
    assert_eq!(
        launched.feature.id.0, "f-1",
        "the executor's Feature is returned"
    );
    assert_eq!(launched.credentials_parked, None);
    assert_eq!(
        h.exec.calls(),
        Vec::<String>::new(),
        "no RPC on a local launch"
    );
    assert_eq!(h.ctx.remote_run_mirror.list().unwrap().len(), 0);
}

#[tokio::test]
async fn a_detached_launch_returns_the_shadow_feature_under_the_chosen_id() {
    let h = harness(RunnerAt::accepting());

    let launched = launch_run(
        &h.ctx,
        request(detached("runner-1"), DetachedOptions::default()),
    )
    .await
    .expect("a detached launch is submitted");

    assert_eq!(launched.feature.id.0, "f-chosen");
    assert_eq!(launched.feature.status, "pending");
    assert_eq!(launched.credentials_parked, None);
    assert_eq!(launched.mirror_unrecorded, None);
    assert_eq!(feature_ids(&h.ctx), ["f-chosen"]);
    let mirrors = h.ctx.remote_run_mirror.list().unwrap();
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].feature_id.as_deref(), Some("f-chosen"));
    assert!(h.spy.launched().is_none(), "the executor is not called");
    assert!(h.exec.calls().contains(&"rpc submit_run".to_string()));
}

/// D8: the runner accepted the run, so it is a launched run whose PAT is
/// still owed — not a failed launch.
#[tokio::test]
async fn a_detached_launch_carries_parked_credentials_through() {
    let h = harness(RunnerAt {
        accepts_credentials: false,
        ..RunnerAt::accepting()
    });

    let launched = launch_run(
        &h.ctx,
        request(detached("runner-1"), DetachedOptions::default()),
    )
    .await
    .expect("an accepted run is a launched run");

    assert_eq!(launched.feature.id.0, "f-chosen");
    let reason = launched.credentials_parked.expect("credentials are parked");
    assert!(reason.contains("inject_credentials"), "{reason}");
    assert!(h.spy.launched().is_none());
}

/// D8: a mirror row the laptop failed to write does not unmake a run the
/// runner accepted. The launch still returns the shadow Feature, says what
/// was not recorded, and still delivers the PAT.
#[tokio::test]
async fn a_mirror_failure_after_acceptance_is_a_launched_run() {
    for write in [MirrorWrite::Submitted, MirrorWrite::Status] {
        let mut h = harness(RunnerAt::accepting());
        h.ctx.remote_run_mirror = MirrorFailingOn::wrap(h.ctx.remote_run_mirror.clone(), write);

        let launched = launch_run(
            &h.ctx,
            request(detached("runner-1"), DetachedOptions::default()),
        )
        .await
        .unwrap_or_else(|error| panic!("{write:?}: an accepted run is launched, got {error}"));

        assert_eq!(launched.feature.id.0, "f-chosen", "{write:?}");
        let reason = launched
            .mirror_unrecorded
            .unwrap_or_else(|| panic!("{write:?}: the mirror failure is named"));
        assert!(reason.contains(write.method()), "{reason}");
        assert_eq!(launched.credentials_parked, None, "{write:?}");
        assert!(
            h.exec
                .calls()
                .contains(&"rpc inject_credentials".to_string()),
            "{write:?}: the PAT is still delivered"
        );
        assert!(h.spy.launched().is_none(), "the executor is not called");
    }
}

// --- AC2 ------------------------------------------------------------------

/// What every refusal has in common: `Err`, no Feature row, no mirror row,
/// and the executor never reached.
async fn refused(h: &Harness, req: LaunchRequest) -> AppError {
    let Err(error) = launch_run(&h.ctx, req).await else {
        panic!("the launch must be refused");
    };
    assert_eq!(feature_ids(&h.ctx), Vec::<String>::new(), "no Feature row");
    assert_eq!(
        h.ctx.remote_run_mirror.list().unwrap().len(),
        0,
        "no mirror row"
    );
    assert!(h.spy.launched().is_none(), "the executor is not called");
    error
}

/// The runner refusals may have asked the machine for its version, and
/// nothing else.
fn assert_probe_only(exec: &RunnerAt) {
    let calls = exec.calls();
    assert!(!calls.is_empty(), "the runner was probed");
    for call in &calls {
        assert!(
            call == "rpc health"
                || call == "resolve_home"
                || (call.starts_with("run_command") && call.contains("--version")),
            "only probe calls before a refusal, got {calls:?}"
        );
    }
}

#[tokio::test]
async fn an_unknown_machine_is_refused_without_an_rpc() {
    let h = harness(RunnerAt::accepting());

    let error = refused(
        &h,
        request(detached("runner-gone"), DetachedOptions::default()),
    )
    .await;

    let message = error.to_string();
    assert!(message.contains("runner-gone"), "{message}");
    assert!(message.contains("Machines settings"), "{message}");
    assert_eq!(h.exec.calls(), Vec::<String>::new());
}

#[tokio::test]
async fn the_desktop_is_refused_as_a_detached_target_without_an_rpc() {
    let h = harness(RunnerAt::accepting());
    h.ctx.machines.add(machine("this-laptop", "local")).unwrap();

    for target in ["local", "this-laptop"] {
        refused(&h, request(detached(target), DetachedOptions::default())).await;
        assert_eq!(h.exec.calls(), Vec::<String>::new(), "{target}");
    }
}

async fn assert_runner_refused(exec: RunnerAt) {
    let h = harness(exec);

    let error = refused(
        &h,
        request(detached("runner-1"), DetachedOptions::default()),
    )
    .await;

    assert_eq!(error.code(), "runner_incompatible", "{error}");
    assert_probe_only(&h.exec);
}

#[tokio::test]
async fn a_machine_with_no_runner_installed_is_refused() {
    assert_runner_refused(RunnerAt::silent(true)).await;
}

#[tokio::test]
async fn a_runner_behind_the_app_is_refused() {
    assert_runner_refused(RunnerAt::reporting("1.2.0-30")).await;
}

#[tokio::test]
async fn a_runner_ahead_of_the_app_is_refused() {
    assert_runner_refused(RunnerAt::reporting("1.2.0-32")).await;
}

#[tokio::test]
async fn an_unreachable_machine_is_refused() {
    assert_runner_refused(RunnerAt::silent(false)).await;
}

#[tokio::test]
async fn an_unset_app_version_is_refused() {
    let mut h = harness(RunnerAt::accepting());
    h.ctx.app_version = crate::state::AppVersion::default();

    let error = refused(
        &h,
        request(detached("runner-1"), DetachedOptions::default()),
    )
    .await;

    assert_eq!(error.code(), "runner_incompatible", "{error}");
    assert_probe_only(&h.exec);
}

#[tokio::test]
async fn a_local_launch_with_a_detached_only_option_is_refused() {
    let options = [
        DetachedOptions {
            target_repo_id: Some("repo-1".to_string()),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            unattended: Some(true),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            max_cost_usd: Some(5.0),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            max_wall_clock_secs: Some(60),
            ..DetachedOptions::default()
        },
    ];
    for opts in options {
        let h = harness(RunnerAt::accepting());

        refused(&h, request(RunPlacement::Local, opts.clone())).await;

        assert_eq!(h.exec.calls(), Vec::<String>::new(), "{opts:?}");
    }
}

#[tokio::test]
async fn an_attended_detached_launch_is_refused() {
    let h = harness(RunnerAt::accepting());

    refused(
        &h,
        request(
            detached("runner-1"),
            DetachedOptions {
                unattended: Some(false),
                ..DetachedOptions::default()
            },
        ),
    )
    .await;

    assert_eq!(h.exec.calls(), Vec::<String>::new());
}

#[tokio::test]
async fn a_detached_launch_with_an_unusable_cap_is_refused() {
    let caps = [
        DetachedOptions {
            max_cost_usd: Some(0.0),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            max_cost_usd: Some(-1.0),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            max_cost_usd: Some(f64::NAN),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            max_cost_usd: Some(f64::INFINITY),
            ..DetachedOptions::default()
        },
        DetachedOptions {
            max_wall_clock_secs: Some(0),
            ..DetachedOptions::default()
        },
    ];
    for opts in caps {
        let h = harness(RunnerAt::accepting());

        refused(&h, request(detached("runner-1"), opts.clone())).await;

        assert_eq!(h.exec.calls(), Vec::<String>::new(), "{opts:?}");
    }
}
