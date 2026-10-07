// Tests for `src/application/remote_runs/submit.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// What a detached launch says about where it starts has to survive two
// separate journeys — onto the laptop's own shadow row, and over the wire —
// and the failure when it survives neither is a run that completes, pushes,
// and opens a PR whose diff is against a tree nobody chose. Nothing observes
// that but the diff, so these assert the two destinations directly.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::Value;

use super::*;
use crate::adapters::notification_noop::NoopNotificationAdapter;
use crate::application::attachments::StagedAttachmentInput;
use crate::application::launch::tests::{MirrorFailingOn, MirrorWrite};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{MachineId, ProviderId, RepositoryId, WorkflowVersionId};
use crate::domain::models::{Machine, Platform, Project, Workflow, WorkflowVersion};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};

const APP_VERSION: &str = "1.2.0-31";

fn submit_input(origin: Option<FeatureOrigin>, diff_base_branch: Option<&str>) -> SubmitInput {
    SubmitInput {
        machine_id: MachineId::from("runner-1"),
        launch: FeatureLaunch {
            project_id: "p-1".to_string(),
            workflow_id: "w-1".to_string(),
            title: "Ship it".to_string(),
            description: "A detached run".to_string(),
            origin: origin.unwrap_or_default(),
            diff_base_branch: diff_base_branch.map(str::to_string),
            ..FeatureLaunch::default()
        },
        detached: DetachedOptions::default(),
    }
}

fn resolved() -> ResolvedSubmit {
    ResolvedSubmit {
        project_id: ProjectId::from("p-1"),
        workflow: ResolvedWorkflow {
            id: WorkflowId::from("w-1"),
            version: WorkflowVersion {
                id: WorkflowVersionId::from("wv-1"),
                workflow_id: WorkflowId::from("w-1"),
                version: 1,
                steps_json: "[]".to_string(),
                definition_json: None,
                created_at: 1_700_000_000,
                note: None,
            },
            json: serde_json::json!({ "name": "W", "description": "", "steps": [] }),
        },
        provider: RunSpecProvider {
            kind: "github".to_string(),
            host: "github.com".to_string(),
        },
        repo_path: "demeteo/demeteo".to_string(),
        project_settings: None,
        attachments: Vec::new(),
        budget: None,
        feature_id: "f-1".to_string(),
        now: 1_700_000_000,
    }
}

fn pr_head() -> FeatureOrigin {
    FeatureOrigin::Ref {
        fetch_spec: "refs/pull/42/head".to_string(),
        label: "PR #42".to_string(),
    }
}

#[test]
fn the_shadow_row_records_the_origin_the_launch_chose() {
    let input = submit_input(Some(pr_head()), Some("release/2.0"));
    let feature = resolved().shadow_feature(&input.launch);

    assert_eq!(feature.origin, pr_head());
    assert_eq!(feature.diff_base_branch.as_deref(), Some("release/2.0"));
}

#[test]
fn the_wire_spec_carries_the_origin_the_launch_chose() {
    let input = submit_input(Some(pr_head()), Some("release/2.0"));
    let spec = resolved().run_spec(&input.launch);

    assert_eq!(spec.origin, Some(RunOrigin::Supported(pr_head())));
    assert_eq!(spec.diff_base_branch.as_deref(), Some("release/2.0"));
}

/// The runner does not read `RunSpec::origin` directly, so the assertion
/// above is only half the claim: what matters is that the JSON it decodes
/// resolves to the same origin rather than to `RunOrigin::Unsupported`, which
/// is what an encoding the runner cannot name looks like from here.
#[test]
fn the_encoded_spec_resolves_on_the_runner_to_the_origin_that_was_sent() {
    let input = submit_input(Some(pr_head()), None);
    let wire = serde_json::to_value(resolved().run_spec(&input.launch)).expect("spec serializes");
    let decoded: RunSpec = serde_json::from_value(wire).expect("spec round-trips");

    assert_eq!(decoded.origin_to_honour(), Ok(pr_head()));
}

/// A launch that named nothing is the pre-V41 launch: the row stores the
/// default branch, and the wire names it rather than leaving it to a default
/// the runner might not share.
#[test]
fn naming_no_origin_submits_the_default_branch() {
    let input = submit_input(None, None);
    let resolved = resolved();

    assert_eq!(
        resolved.shadow_feature(&input.launch).origin,
        FeatureOrigin::DefaultBranch
    );
    let spec = resolved.run_spec(&input.launch);
    assert_eq!(
        spec.origin,
        Some(RunOrigin::Supported(FeatureOrigin::DefaultBranch))
    );
    assert_eq!(spec.origin_to_honour(), Ok(FeatureOrigin::DefaultBranch));
}

#[test]
fn the_shadow_row_and_the_wire_agree_on_where_the_run_starts() {
    let input = submit_input(
        Some(FeatureOrigin::Branch {
            base: "release/2.0".to_string(),
        }),
        None,
    );
    let resolved = resolved();

    assert_eq!(
        resolved.run_spec(&input.launch).origin_to_honour(),
        Ok(resolved.shadow_feature(&input.launch).origin)
    );
}

/// A runner reporting `build_version` over health, and a machine on which
/// spooling an attachment would *succeed* — so a gate that ran after the
/// spool is caught by the calls it made, not by an error it happened to hit.
/// `submit_run` is accepted only when built with [`RunnerAt::accepting`];
/// everything else is `Err`.
struct RunnerAt {
    build_version: &'static str,
    accepts_submit: bool,
    calls: Mutex<Vec<String>>,
    submitted: Mutex<Option<Value>>,
}

impl RunnerAt {
    fn new(build_version: &'static str) -> Self {
        Self {
            build_version,
            accepts_submit: false,
            calls: Mutex::new(Vec::new()),
            submitted: Mutex::new(None),
        }
    }

    fn accepting(build_version: &'static str) -> Self {
        Self {
            accepts_submit: true,
            ..Self::new(build_version)
        }
    }

    fn submitted_spec(&self) -> Value {
        self.submitted
            .lock()
            .unwrap()
            .clone()
            .expect("submit_run was sent")["spec"]
            .clone()
    }

    fn record(&self, call: String) {
        self.calls.lock().unwrap().push(call);
    }

    fn calls(&self) -> Vec<String> {
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
        Ok(String::new())
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
        path: &str,
        _content: &[u8],
    ) -> Result<(), String> {
        self.record(format!("write_file_bytes {path}"));
        Ok(())
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
        self.record("resolve_home".to_string());
        Ok("/home/runner".to_string())
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
        self.record(format!("rpc {method}"));
        match method {
            "health" => Ok(serde_json::json!({ "build_version": self.build_version })),
            "submit_run" if self.accepts_submit => {
                *self.submitted.lock().unwrap() = Some(params);
                Ok(serde_json::json!({ "status": "pending" }))
            }
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

fn machine(id: &str, auth_type: &str) -> Machine {
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

/// A context in which every step before the RPC would succeed: the project,
/// its repository and provider, the workflow, and a PAT seeded into the
/// process-wide credential cache under a provider id no other test uses.
fn submittable_ctx(exec: Arc<RunnerAt>) -> (AppContext, crate::support::test_dir::TestDir) {
    let dir = crate::support::test_dir::TestDir::new("demeteo-submit-gate");
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir.path().to_path_buf(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    ctx.exec = exec;
    ctx.app_version.set(APP_VERSION.to_string());
    ctx.machines.add(machine("runner-1", "key")).unwrap();
    ctx.projects
        .add(Project {
            id: ProjectId::from("p-1"),
            name: "gated".to_string(),
            compute_type: "remote".to_string(),
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
            id: ProviderId::from("prov-submit-gate"),
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
            provider_id: ProviderId::from("prov-submit-gate"),
            repo_path: "org/repo".to_string(),
        })
        .unwrap();
    crate::credential_cache::set("prov-submit-gate", "pat-never-sent");
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
        .save_version(resolved().workflow.version)
        .unwrap();
    (ctx, dir)
}

/// D7: a refused submit has touched nothing — no attachment on the runner's
/// disk, no placeholder row, and the machine asked only for its version.
#[tokio::test]
async fn a_mismatched_runner_is_refused_before_any_side_effect() {
    let exec = Arc::new(RunnerAt::new("1.2.0-30"));
    let (ctx, _dir) = submittable_ctx(exec.clone());
    let mut input = submit_input(None, None);
    input.launch.staged_attachments = vec![StagedAttachmentInput {
        source_path: String::new(),
        mime: Some("text/plain".to_string()),
        source_filename: Some("notes.txt".to_string()),
        bytes: Some(b"spooled only if the gate ran late".to_vec()),
    }];

    let result = submit_remote_run(&ctx, input).await;

    assert!(
        matches!(result, Err(AppError::RunnerIncompatible { .. })),
        "expected RunnerIncompatible, got {:?}",
        result.err()
    );
    assert_eq!(exec.calls(), ["rpc health"]);
    assert_eq!(
        ctx.features
            .get_all_for_project(&ProjectId::from("p-1"))
            .unwrap()
            .len(),
        0,
        "a refused submit leaves no shadow feature row"
    );
}

/// The gate's other half: this build's runner is let through to the submit
/// itself, which the double then refuses.
#[tokio::test]
async fn a_runner_on_this_build_is_let_through_to_the_submit() {
    let exec = Arc::new(RunnerAt::new(APP_VERSION));
    let (ctx, _dir) = submittable_ctx(exec.clone());

    let result = submit_remote_run(&ctx, submit_input(None, None)).await;

    assert!(
        !matches!(result, Err(AppError::RunnerIncompatible { .. })),
        "a matching runner must not be refused"
    );
    assert_eq!(exec.calls(), ["rpc health", "rpc submit_run"]);
}

/// D7: nothing about the machine has been asked over the wire, and nothing
/// was written here, when the id cannot receive a detached run at all.
async fn assert_refused_untouched(ctx: &AppContext, exec: &RunnerAt, machine_id: &str) -> String {
    let mut input = submit_input(None, None);
    input.machine_id = MachineId::from(machine_id);

    let result = submit_remote_run(ctx, input).await;

    let Err(error) = result else {
        panic!("submitting to `{machine_id}` must be refused");
    };
    assert_eq!(
        exec.calls(),
        Vec::<String>::new(),
        "no call reaches the machine"
    );
    assert_eq!(
        ctx.features
            .get_all_for_project(&ProjectId::from("p-1"))
            .unwrap()
            .len(),
        0,
        "a refused submit leaves no shadow feature row"
    );
    assert_eq!(ctx.remote_run_mirror.list().unwrap().len(), 0);
    error.to_string()
}

#[tokio::test]
async fn an_unknown_machine_is_refused_before_any_rpc() {
    let exec = Arc::new(RunnerAt::new(APP_VERSION));
    let (ctx, _dir) = submittable_ctx(exec.clone());

    let message = assert_refused_untouched(&ctx, &exec, "runner-gone").await;

    assert!(message.contains("runner-gone"), "{message}");
    assert!(message.contains("Machines settings"), "{message}");
}

#[tokio::test]
async fn the_local_id_is_refused_before_any_rpc() {
    let exec = Arc::new(RunnerAt::new(APP_VERSION));
    let (ctx, _dir) = submittable_ctx(exec.clone());

    assert_refused_untouched(&ctx, &exec, "local").await;
}

#[tokio::test]
async fn a_machine_row_for_the_desktop_is_refused_before_any_rpc() {
    let exec = Arc::new(RunnerAt::new(APP_VERSION));
    let (ctx, _dir) = submittable_ctx(exec.clone());
    ctx.machines.add(machine("this-laptop", "local")).unwrap();

    assert_refused_untouched(&ctx, &exec, "this-laptop").await;
}

/// The version comes from the context, and a context nobody set it on must
/// not read as compatible with whatever runner answers.
#[tokio::test]
async fn an_unset_app_version_is_refused_before_the_submit() {
    let exec = Arc::new(RunnerAt::new(APP_VERSION));
    let (mut ctx, _dir) = submittable_ctx(exec.clone());
    ctx.app_version = crate::state::AppVersion::default();

    let result = submit_remote_run(&ctx, submit_input(None, None)).await;

    let Err(error) = result else {
        panic!("an unset app version must refuse the submit");
    };
    assert_eq!(error.code(), "runner_incompatible");
    assert!(!exec.calls().iter().any(|call| call == "rpc submit_run"));
}

fn shadow_ids(ctx: &AppContext) -> Vec<String> {
    ctx.features
        .get_all_for_project(&ProjectId::from("p-1"))
        .unwrap()
        .into_iter()
        .map(|feature| feature.id.0)
        .collect()
}

/// A ticket's run already knows the Feature id it records its attempt
/// against, so the shadow row, the mirror and the wire must all reuse it.
#[tokio::test]
async fn a_supplied_feature_id_is_the_shadow_and_mirror_id() {
    let exec = Arc::new(RunnerAt::accepting(APP_VERSION));
    let (ctx, _dir) = submittable_ctx(exec.clone());
    let mut input = submit_input(None, None);
    input.launch.feature_id = Some("f-ticket-7".to_string());

    let outcome = submit_remote_run(&ctx, input).await.unwrap();

    assert_eq!(outcome.feature_id, "f-ticket-7");
    assert_eq!(shadow_ids(&ctx), ["f-ticket-7"]);
    let mirrors = ctx.remote_run_mirror.list().unwrap();
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].feature_id.as_deref(), Some("f-ticket-7"));
    assert_eq!(exec.submitted_spec()["feature_id"], "f-ticket-7");
}

/// D6: nobody is attached to a detached run, so the request never asks for
/// an attended one — whatever the caller left unset.
#[tokio::test]
async fn every_submitted_run_is_unattended() {
    for unattended in [None, Some(true)] {
        let exec = Arc::new(RunnerAt::accepting(APP_VERSION));
        let (ctx, _dir) = submittable_ctx(exec.clone());
        let mut input = submit_input(None, None);
        input.detached.unattended = unattended;

        let _ = submit_remote_run(&ctx, input).await;

        assert_eq!(exec.submitted_spec()["unattended"], true, "{unattended:?}");
    }
}

/// D8: once the runner has accepted the run it exists, PAT or not. Reporting
/// the launch as failed would let a ticket submit it a second time.
#[tokio::test]
async fn a_credential_failure_after_an_accepted_submit_parks_the_run() {
    let exec = Arc::new(RunnerAt::accepting(APP_VERSION));
    let (ctx, _dir) = submittable_ctx(exec.clone());

    let outcome = submit_remote_run(&ctx, submit_input(None, None))
        .await
        .expect("an accepted run is a started run");

    assert_eq!(
        exec.calls(),
        ["rpc health", "rpc submit_run", "rpc inject_credentials"]
    );
    let reason = outcome.credentials_parked.expect("credentials are parked");
    assert!(reason.contains("inject_credentials"), "{reason}");
    assert_eq!(outcome.mirror_unrecorded, None);
    assert_eq!(shadow_ids(&ctx), std::slice::from_ref(&outcome.feature_id));
    let mirrors = ctx.remote_run_mirror.list().unwrap();
    assert_eq!(mirrors.len(), 1);
    assert_eq!(mirrors[0].run_id, outcome.run_id);
    assert_eq!(mirrors[0].feature_id.as_deref(), Some(&*outcome.feature_id));
}

/// D8 again, for the laptop's own bookkeeping: a mirror write failing after
/// the runner accepted the run is named on the outcome, not raised, and the
/// PAT is still offered to the run.
#[tokio::test]
async fn a_mirror_failure_after_an_accepted_submit_is_named_not_raised() {
    for write in [MirrorWrite::Submitted, MirrorWrite::Status] {
        let exec = Arc::new(RunnerAt::accepting(APP_VERSION));
        let (mut ctx, _dir) = submittable_ctx(exec.clone());
        ctx.remote_run_mirror = MirrorFailingOn::wrap(ctx.remote_run_mirror.clone(), write);

        let outcome = submit_remote_run(&ctx, submit_input(None, None))
            .await
            .unwrap_or_else(|error| {
                panic!("{write:?}: an accepted run is a started run, got {error}")
            });

        let reason = outcome
            .mirror_unrecorded
            .unwrap_or_else(|| panic!("{write:?}: the mirror failure is named"));
        assert!(reason.contains(write.method()), "{reason}");
        assert_eq!(
            exec.calls(),
            ["rpc health", "rpc submit_run", "rpc inject_credentials"],
            "{write:?}"
        );
        assert_eq!(shadow_ids(&ctx), std::slice::from_ref(&outcome.feature_id));
    }
}
