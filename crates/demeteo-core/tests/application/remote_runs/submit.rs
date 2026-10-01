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
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{ProviderId, RepositoryId, WorkflowVersionId};
use crate::domain::models::{Platform, Project, Workflow, WorkflowVersion};
use crate::ports::execution::{ExecutionPort, InteractiveHandle, SftpEntry};

const APP_VERSION: &str = "1.2.0-31";

fn submit_input(origin: Option<FeatureOrigin>, diff_base_branch: Option<&str>) -> SubmitInput {
    SubmitInput {
        machine_id: "runner-1".to_string(),
        project_id: "p-1".to_string(),
        workflow_id: "w-1".to_string(),
        title: "Ship it".to_string(),
        description: "A detached run".to_string(),
        agent_kind: None,
        model: None,
        effort: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: None,
        staged_attachments: None,
        target_repo_id: None,
        unattended: false,
        max_cost_usd: None,
        max_wall_clock_secs: None,
        origin,
        diff_base_branch: diff_base_branch.map(str::to_string),
        app_version: APP_VERSION.to_string(),
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
        step_overrides: Vec::new(),
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
    let feature = resolved().shadow_feature(&input);

    assert_eq!(feature.origin, pr_head());
    assert_eq!(feature.diff_base_branch.as_deref(), Some("release/2.0"));
}

#[test]
fn the_wire_spec_carries_the_origin_the_launch_chose() {
    let input = submit_input(Some(pr_head()), Some("release/2.0"));
    let spec = resolved().run_spec(&input);

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
    let wire = serde_json::to_value(resolved().run_spec(&input)).expect("spec serializes");
    let decoded: RunSpec = serde_json::from_value(wire).expect("spec round-trips");

    assert_eq!(decoded.origin_to_honour(), Ok(pr_head()));
}

/// A launch that named nothing is the pre-V41 launch, and both records have
/// to say so in their own spelling: the row stores the default branch, the
/// wire stays absent so a runner that predates the field is unaffected.
#[test]
fn naming_no_origin_submits_the_default_branch() {
    let input = submit_input(None, None);
    let resolved = resolved();

    assert_eq!(
        resolved.shadow_feature(&input).origin,
        FeatureOrigin::DefaultBranch
    );
    let spec = resolved.run_spec(&input);
    assert_eq!(spec.origin, None);
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
        resolved.run_spec(&input).origin_to_honour(),
        Ok(resolved.shadow_feature(&input).origin)
    );
}

/// A runner reporting `build_version` over health, and a machine on which
/// spooling an attachment would *succeed* — so a gate that ran after the
/// spool is caught by the calls it made, not by an error it happened to hit.
/// Everything else is `Err`.
struct RunnerAt {
    build_version: &'static str,
    calls: Mutex<Vec<String>>,
}

impl RunnerAt {
    fn new(build_version: &'static str) -> Self {
        Self {
            build_version,
            calls: Mutex::new(Vec::new()),
        }
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
        _params: Value,
    ) -> Result<Value, String> {
        self.record(format!("rpc {method}"));
        match method {
            "health" => Ok(serde_json::json!({ "build_version": self.build_version })),
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

/// A context in which every step before the RPC would succeed: the project,
/// its repository and provider, the workflow, and a PAT seeded into the
/// process-wide credential cache under a provider id no other test uses.
fn submittable_ctx(exec: Arc<RunnerAt>) -> (AppContext, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("demeteo-submit-gate-{}", crate::paths::new_id()));
    let mut ctx = build_core_context(
        CoreConfig {
            app_data_dir: dir.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotificationAdapter),
        tokio::runtime::Handle::current(),
    );
    ctx.exec = exec;
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
    let (ctx, dir) = submittable_ctx(exec.clone());
    let mut input = submit_input(None, None);
    input.staged_attachments = Some(vec![StagedAttachmentInput {
        source_path: String::new(),
        mime: Some("text/plain".to_string()),
        source_filename: Some("notes.txt".to_string()),
        bytes: Some(b"spooled only if the gate ran late".to_vec()),
    }]);

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

    let _ = std::fs::remove_dir_all(dir);
}

/// The gate's other half: this build's runner is let through to the submit
/// itself, which the double then refuses.
#[tokio::test]
async fn a_runner_on_this_build_is_let_through_to_the_submit() {
    let exec = Arc::new(RunnerAt::new(APP_VERSION));
    let (ctx, dir) = submittable_ctx(exec.clone());

    let result = submit_remote_run(&ctx, submit_input(None, None)).await;

    assert!(
        !matches!(result, Err(AppError::RunnerIncompatible { .. })),
        "a matching runner must not be refused"
    );
    assert_eq!(exec.calls(), ["rpc health", "rpc submit_run"]);

    let _ = std::fs::remove_dir_all(dir);
}
