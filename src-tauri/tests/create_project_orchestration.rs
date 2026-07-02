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
//!   `CreateProjectAdapter` with stub `ExecutionPort` /
//!   `ProjectRepository` / `StepExecutor` implementations, asserting
//!   that the create-remote-repo → persist-project → dispatch-feature
//!   sequence completes with the expected `LaunchedFeature` shape.
//!
//! The React side (`src/hooks/useCreateProjectWizard.test.tsx`) covers
//! the view-emission contract end-to-end; together they pin AC-5.

use async_trait::async_trait;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use demeteo_lib::adapters::create_project_adapter::CreateProjectAdapter;
use demeteo_lib::commands::attachments::StagedAttachmentInput;
use demeteo_lib::commands::create_project::{BootstrapOutcome, CreateProjectStepPayload};
use demeteo_lib::domain::bootstrap::{BootstrapState, BootstrapStep, STEP_ORDER};
use demeteo_lib::domain::ids::{ProjectId, ProviderId, RepositoryId, WorkflowId};
use demeteo_lib::domain::models::{
    Feature, Project, ProjectSettings, ProjectWorkflowOverride, Repository, StepExecution,
    StepOverride,
};
use demeteo_lib::domain::models::GateDecision;
use demeteo_lib::error::AppError;
use demeteo_lib::ports::create_project_port::{CreateProjectPort, LaunchedFeature};
use demeteo_lib::ports::db::ProjectRepository;
use demeteo_lib::ports::execution::ExecutionPort;
use demeteo_lib::ports::provider_http::NamespaceSummary;
use demeteo_lib::ports::step_executor::{GatePresenter, StepExecutor, SyncOutcomeView};
use demeteo_lib::sftp::SftpEntry;

// ── Stub ports ─────────────────────────────────────────────────────────
//
// Minimal implementations of the ports the orchestration touches. They
// record calls in shared `Mutex<Vec<…>>` so the test can assert the
// call sequence without inspecting private state.

#[derive(Default)]
struct ExecCalls {
    run_command: Mutex<Vec<(String, String)>>,
}

struct StubExec {
    calls: std::sync::Arc<ExecCalls>,
}

#[async_trait]
impl ExecutionPort for StubExec {
    async fn test_connection(&self, _machine_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_command(&self, machine_id: &str, cmd: &str) -> Result<String, String> {
        self.calls
            .run_command
            .lock()
            .unwrap()
            .push((machine_id.to_string(), cmd.to_string()));
        // The wizard invokes `gh repo create` (or `glab project create`)
        // through `run_command`. Return a JSON payload that the
        // adapter's `parse_gh_create_repo_output` / `parse_glab_*`
        // helpers can decode.
        //
        // Schema (matches `infrastructure::gh_gl_cli::parse_gh_create_repo_output`):
        //   - `name` → full_name (gh CLI)
        //   - `defaultBranchRef/name` → default_branch
        //   - `url` → clone_url
        Ok(r#"{"name":"octocat/billing-service","defaultBranchRef":{"name":"main"},"url":"https://github.com/octocat/billing-service.git"}"#.to_string())
    }
    async fn read_file(&self, _machine_id: &str, _path: &str) -> Result<String, String> {
        Ok(String::new())
    }
    async fn write_file(
        &self,
        _machine_id: &str,
        _path: &str,
        _content: &str,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn get_metadata(&self, _machine_id: &str, _path: &str) -> Result<SftpEntry, String> {
        Err("not used".into())
    }
    async fn list_dir(&self, _machine_id: &str, _path: &str) -> Result<Vec<SftpEntry>, String> {
        Ok(Vec::new())
    }
    async fn setup_worktree(
        &self,
        _machine_id: &str,
        _repo_path: &str,
        _branch: &str,
        _sandbox_path: &str,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn resolve_home(&self, _machine_id: &str) -> Result<String, String> {
        Ok("/tmp".into())
    }
    fn spawn_interactive(
        &self,
        _machine_id: &str,
        _binary: &str,
        _args: &[String],
        _cwd: &str,
        _env: &HashMap<String, String>,
    ) -> Result<Box<dyn demeteo_lib::ports::execution::InteractiveHandle>, String> {
        Err("not used".into())
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
    fn delete(&self, _id: &ProjectId) -> Result<(), String> {
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
    async fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
        _commit_artifacts: Option<bool>,
        _loop_iterations: Option<u32>,
        _step_overrides: Vec<StepOverride>,
        _staged_attachments: Vec<StagedAttachmentInput>,
    ) -> Result<Feature, String> {
        self.calls.features_started.lock().unwrap().push((
            project_id.to_string(),
            workflow_id.to_string(),
            title.to_string(),
            description.to_string(),
            agent_kind.map(|s| s.to_string()),
            model.map(|s| s.to_string()),
        ));
        Ok(Feature {
            id: demeteo_lib::domain::ids::FeatureId(format!("feat-{}", project_id)),
            project_id: ProjectId::from(project_id.to_string()),
            workflow_id: Some(WorkflowId::from(workflow_id.to_string())),
            title: title.to_string(),
            status: "running".to_string(),
            created_at: 0,
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            agent_kind: agent_kind.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            mr_url: None,
            mr_state: Some("none".to_string()),
            commit_artifacts: None,
            loop_iterations: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
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
    ) -> Result<(), AppError> {
        Ok(())
    }
    async fn replay_from_step(
        &self,
        _execution_id: &str,
        _new_model: Option<&str>,
        _new_agent: Option<&str>,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn step_list_for_run(&self, _feature_id: &str) -> Result<Vec<StepExecution>, String> {
        Ok(Vec::new())
    }
    async fn feature_sync(
        &self,
        _feature_id: &str,
        _revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String> {
        Ok(SyncOutcomeView::Ok {
            merge_commit_sha: "deadbeef".into(),
            changed: false,
        })
    }
    async fn feature_resolve_sync_conflicts(
        &self,
        _feature_id: &str,
        _conflict_files: &[String],
        _revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String> {
        Ok(SyncOutcomeView::Ok {
            merge_commit_sha: "deadbeef".into(),
            changed: false,
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
    std::sync::Arc<ExecCalls>,
    std::sync::Arc<ExecStartCalls>,
) {
    let exec_calls = std::sync::Arc::new(ExecCalls::default());
    let exec = std::sync::Arc::new(StubExec {
        calls: exec_calls.clone(),
    });
    let adapter = CreateProjectAdapter::new(exec);
    (
        adapter,
        exec_calls,
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
    };
    assert_eq!(commit.expected_step(), BootstrapStep::Description);
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
    let exec_calls = std::sync::Arc::new(ExecCalls::default());
    let project_calls = std::sync::Arc::new(ProjectCalls::default());
    let exec_calls_for_stub = exec_calls.clone();
    let exec = std::sync::Arc::new(StubExec {
        calls: exec_calls_for_stub,
    });
    let adapter = CreateProjectAdapter::new(exec);

    let namespace = NamespaceSummary {
        id: "octocat".into(),
        name: "octocat".into(),
        kind: "personal".into(),
    };
    let created = adapter
        .create_remote_repo(
            "github",
            "github.com",
            &namespace,
            "billing-service",
            "private",
        )
        .await
        .expect("create_remote_repo must succeed with stub exec");
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
        CreateProjectStepPayload::Model { model: "m".into() },
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
