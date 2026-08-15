//! `command` node end-to-end gate (task P3.5, PRD §5.2 / Decision 8).
//!
//! The task's Done-when is *"a starter-style workflow with a command node
//! runs under the stub harness"*. Three runs prove the node type is real
//! rather than merely registered:
//!
//! 1. **It runs.** An agent step feeds a command step; the command's
//!    stdout lands as an artifact, its declared `last_write_to`
//!    deliverable is read back off the worktree, and the feature reaches
//!    a terminal success — all at zero token cost.
//! 2. **It fails like a harness.** A non-zero exit fails the step with the
//!    command's own output as the reason, classified `verdict` — the class
//!    the P1.10 retry policy redirects to an implementation step, which is
//!    what makes `baseline-harness(command)` (PRD §7) useful.
//! 3. **It respects the idempotency rule.** A command declared
//!    `idempotent: false` that was interrupted parks at the Decision-14
//!    synthetic gate *even though the workspace fingerprint matches* —
//!    the P1.14 guard cannot see what a deploy did outside the worktree,
//!    so it asks. This is the case the fingerprint alone gets wrong, and
//!    the reason [`ResumePolicy`](crate::adapters::step_executor::registry::ResumePolicy)
//!    exists.
//!
//! Nothing here touches the scheduler or the driver's dispatch: the node
//! type reached the engine through one registration line.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::{ProviderInstance, StepExecution};
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/command-node";
const PROVIDER_ID: &str = "command-node-provider";
const BUILD_LOG: &str = "artifacts/build-log.txt";
const STDOUT_MARKER: &str = "DEMETEO_COMMAND_NODE_RAN";

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// Starter-shaped: an agent step produces a report, then a zero-token
/// command step runs the "harness". `command` is the shell the author
/// wrote; nothing about it is Demeteo-specific.
fn workflow_with_command(command: &str, idempotent: bool) -> serde_json::Value {
    serde_json::json!({
        "name": "Command Node Pipeline",
        "description": "Agent step + command step for the P3.5 gate.",
        "steps": [
            {
                "id": "s-research",
                "kind": "agent",
                "title": "Research",
                "agent_kind": "stub",
                "prompt_template":
                    "Write the research note.\n@stub-write artifacts/research.md\n",
                "capability": "artifacts",
                "artifacts": [
                    {
                        "name": "research",
                        "capture": { "kind": "last_write_to", "path": "artifacts/research.md" },
                        "mode": "full"
                    }
                ]
            },
            {
                "id": "s-harness",
                "kind": "command",
                "title": "Baseline harness",
                "command": command,
                "idempotent": idempotent,
                "capability": "verify",
                "artifacts": [
                    {
                        "name": "build-log",
                        "capture": { "kind": "last_write_to", "path": BUILD_LOG },
                        "mode": "full"
                    }
                ]
            }
        ]
    })
}

/// A command node with no declared deliverables — used where the test
/// only cares about the exit status.
fn workflow_with_bare_command(command: &str, idempotent: bool) -> serde_json::Value {
    serde_json::json!({
        "name": "Bare Command Pipeline",
        "description": "Single command step for the P3.5 gate.",
        "steps": [
            {
                "id": "s-harness",
                "kind": "command",
                "title": "Harness",
                "command": command,
                "idempotent": idempotent,
                "capability": "verify"
            }
        ]
    })
}

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_local_repo(workspace_dir: &Path, project_id: &str) -> PathBuf {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, REPO_PATH);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    git(&dir, &["init", "-b", "main"]);
    git(&dir, &["config", "user.email", "demeteo@local"]);
    git(&dir, &["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# command node fixture\n").expect("seed README");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-m", "seed"]);
    dir
}

async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = ctx
            .features
            .get(feature_id)
            .ok()
            .flatten()
            .map(|f| f.status)
            .unwrap_or_default();
        if matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "awaiting_mr"
        ) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "feature did not settle (last: {status})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn ctx_for(dir: &Path) -> AppContext {
    build_core_context(
        CoreConfig {
            app_data_dir: dir.to_path_buf(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    )
}

fn fresh_dir(tag: &str) -> PathBuf {
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-command-node-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");
    tmp
}

/// Seed a project + workflow and start a feature. Returns the context,
/// data dir, feature id, and the repo working directory.
async fn start_feature(
    tag: &str,
    definition: &serde_json::Value,
) -> (AppContext, PathBuf, FeatureId, PathBuf) {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = fresh_dir(tag);
    let ctx = ctx_for(&tmp);

    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from(PROVIDER_ID),
            kind: "github".to_string(),
            host: "github.com".to_string(),
            username: String::new(),
            avatar_url: String::new(),
            created_at: paths::now_ms(),
        })
        .expect("register provider");

    let project = projects::create(
        &ctx,
        projects::ProjectConfig {
            name: format!("command-node-{tag}"),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    let repo_dir = init_local_repo(&ctx.workspace_dir, project.id.as_str());
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");
    let workflow_id =
        workflows::create_from_json(&ctx.workflows, definition).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(FeatureLaunch {
            project_id: project.id.0.clone(),
            workflow_id: workflow_id.0.clone(),
            title: "Command node run".to_string(),
            description: "Exercise the command node under the stub harness.".to_string(),
            agent_kind: Some("stub".to_string()),
            ..Default::default()
        })
        .await
        .expect("feature_start");

    (ctx, tmp, feature.id, repo_dir)
}

fn step(ctx: &AppContext, feature_id: &FeatureId, step_id: &str) -> StepExecution {
    ctx.features
        .steps_for_feature(feature_id)
        .expect("steps")
        .into_iter()
        .find(|s| s.step_id.0 == step_id)
        .unwrap_or_else(|| panic!("{step_id} row exists"))
}

/// Artifact bodies for a step, keyed by file basename.
async fn artifact_bodies(
    ctx: &AppContext,
    step: &StepExecution,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for path in &step.artifact_paths {
        let body = ctx
            .run_view
            .artifact_body("local", path)
            .await
            .unwrap_or_else(|e| panic!("read artifact {path}: {e}"));
        let base = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path)
            .to_string();
        out.insert(base, body);
    }
    out
}

// ── 1. It runs ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn command_node_runs_and_captures_its_evidence() {
    // Writes the declared deliverable, then echoes a marker to stdout —
    // the two channels the node captures.
    let cmd = format!("mkdir -p artifacts && echo built > {BUILD_LOG} && echo {STDOUT_MARKER}");
    let (ctx, tmp, feature_id, _repo) =
        start_feature("runs", &workflow_with_command(&cmd, true)).await;

    let status = poll_terminal(&ctx, &feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "the run must succeed; got {status}"
    );

    let harness = step(&ctx, &feature_id, "s-harness");
    assert_eq!(harness.status, "completed");
    assert_eq!(harness.step_kind, "command");
    // Zero tokens is the point of the node type (PRD §5.2).
    assert_eq!(harness.cost_usd, Some(0.0), "a command spends no money");
    assert_eq!(harness.tokens, Some(0), "a command spends no tokens");

    let artifacts = artifact_bodies(&ctx, &harness).await;
    let stdout = artifacts
        .iter()
        .find(|(name, _)| name.starts_with("command-output"))
        .map(|(_, body)| body.clone())
        .expect("stdout is always captured");
    assert!(
        stdout.contains(STDOUT_MARKER),
        "stdout artifact should hold the command's output, got: {stdout}"
    );
    let log = artifacts
        .iter()
        .find(|(name, _)| name.starts_with("build-log"))
        .map(|(_, body)| body.clone())
        .expect("the declared last_write_to deliverable is read back off the worktree");
    assert_eq!(log.trim(), "built");

    // The step ran per-attempt telemetry like every other node type (V31).
    let attempts = ctx
        .features
        .attempts_for_step(&harness.id)
        .expect("attempts");
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].status, "completed");

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── 2. It fails like a harness ───────────────────────────────────────────────

#[tokio::test]
async fn a_non_zero_exit_fails_the_step_as_a_verdict() {
    let (ctx, tmp, feature_id, _repo) = start_feature(
        "fails",
        &workflow_with_bare_command("echo boom >&2 && exit 3", true),
    )
    .await;

    let status = poll_terminal(&ctx, &feature_id).await;
    assert_eq!(status, "failed", "a red command must fail the feature");

    let harness = step(&ctx, &feature_id, "s-harness");
    assert_eq!(harness.status, "failed");
    let reason = harness.error_message.unwrap_or_default();
    assert!(
        reason.contains("boom"),
        "the command's own output is the failure reason, got: {reason}"
    );

    // Classified `verdict`, which is what makes the retry policy's
    // redirect-to-implement rule apply to a red harness command.
    let attempts = ctx
        .features
        .attempts_for_step(&harness.id)
        .expect("attempts");
    let last = attempts.last().expect("one attempt");
    assert_eq!(
        last.error_class.as_deref(),
        Some(crate::domain::models::step_attempt::error_class::VERDICT)
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn a_declaration_the_command_never_produced_fails_the_step() {
    // Exits 0 but writes nothing: the deliverable is the point of the step.
    let (ctx, tmp, feature_id, _repo) =
        start_feature("missing", &workflow_with_command("true", true)).await;

    let status = poll_terminal(&ctx, &feature_id).await;
    assert_eq!(status, "failed");

    let harness = step(&ctx, &feature_id, "s-harness");
    let reason = harness.error_message.unwrap_or_default();
    assert!(
        reason.contains("build-log"),
        "the missing deliverable must be named, got: {reason}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

// ── 3. The idempotency rule ──────────────────────────────────────────────────

/// Forge the rows a driver killed mid-command leaves behind: an open
/// attempt recording the *current* (clean) fingerprint, plus step and
/// feature `running`. The fingerprint deliberately **matches** — that is
/// the whole point: a fingerprint-only guard would auto-resume here.
async fn forge_interrupted_command(
    ctx: &AppContext,
    feature_id: &FeatureId,
    repo_dir: &Path,
) -> String {
    git(repo_dir, &["add", "-A"]);
    git(repo_dir, &["commit", "-m", "settle", "--allow-empty"]);

    let harness = step(ctx, feature_id, "s-harness");
    let fp = crate::adapters::step_executor::setup::workspace_fingerprint(
        &*ctx.exec,
        "local",
        &repo_dir.to_string_lossy(),
    )
    .await
    .expect("probe fingerprint");
    assert!(fp.ends_with(":clean"), "settled tree must be clean: {fp}");

    ctx.features
        .attempt_open(&harness.id, paths::now_ms(), Some(&fp))
        .expect("forge open attempt");
    ctx.features
        .step_update(
            &harness.id,
            &StepExecutionPatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("forge step running");
    ctx.features
        .update(
            feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("forge feature running");
    harness.id.0.clone()
}

#[tokio::test]
async fn a_non_idempotent_command_parks_on_resume_despite_a_matching_fingerprint() {
    // `idempotent: false` — a deploy, a publish: nothing it did is visible
    // in the worktree, so the fingerprint has no opinion worth trusting.
    let (ctx, tmp, feature_id, repo_dir) = start_feature(
        "non-idempotent",
        &workflow_with_bare_command(&format!("echo {STDOUT_MARKER}"), false),
    )
    .await;
    let status = poll_terminal(&ctx, &feature_id).await;
    assert!(matches!(status.as_str(), "completed" | "awaiting_mr"));
    let step_exec_id = forge_interrupted_command(&ctx, &feature_id, &repo_dir).await;
    drop(ctx);

    // Life 2: the watchdog marks the step interrupted and the driver arms.
    let ctx2 = ctx_for(&tmp);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let harness = step(&ctx2, &feature_id, "s-harness");
    assert_eq!(
        harness.status, "interrupted",
        "a non-idempotent command must never be re-run on a fingerprint match"
    );
    let feature = ctx2
        .features
        .get(&feature_id)
        .expect("feature read")
        .expect("feature exists");
    assert_eq!(feature.status, "awaiting_gate");

    // A human blesses the re-run, and only then does it happen.
    ctx2.presenter
        .gate_decide(&step_exec_id, "approve", None)
        .await
        .expect("approve synthetic gate");
    let status = poll_terminal(&ctx2, &feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "approval resumes the run; got {status}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

#[tokio::test]
async fn an_idempotent_command_auto_resumes_like_any_other_node() {
    // Control: the same forged crash, but the author declared the command
    // safe to repeat, so P1.14's fingerprint match still governs.
    let (ctx, tmp, feature_id, repo_dir) = start_feature(
        "idempotent",
        &workflow_with_bare_command(&format!("echo {STDOUT_MARKER}"), true),
    )
    .await;
    let status = poll_terminal(&ctx, &feature_id).await;
    assert!(matches!(status.as_str(), "completed" | "awaiting_mr"));
    let _ = forge_interrupted_command(&ctx, &feature_id, &repo_dir).await;
    drop(ctx);

    let ctx2 = ctx_for(&tmp);
    let status = poll_terminal(&ctx2, &feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "a matching fingerprint must auto-resume an idempotent command; got {status}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
