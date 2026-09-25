//! Resume fingerprint guard gate (task P1.14, `docs/TASKS_DAG_WORKFLOWS.md`).
//!
//! The Done-when for P1.14: *a dirty-worktree mutation between crash and
//! resume yields a synthetic gate, not re-execution.* Two lives over one
//! data dir, like the P1.9 crash-resume suite:
//!
//! 1. Life 1 runs a single-step stub feature to completion, then the
//!    test forges the exact rows a driver killed mid-step leaves behind
//!    (step + feature `running`, an open `step_attempts` row recording
//!    the workspace fingerprint at "node start").
//! 2. Life 2 (`build_core_context` again) runs the watchdog (interrupted
//!    + synthetic gate) and auto-arms the driver.
//!    * **mutated** workspace → the P1.14 guard parks: the step must
//!      still be `interrupted` after a grace window, and only a human
//!      `gate_decide("approve")` lets it re-run to completion;
//!    * **untouched** workspace → fingerprints match and the run
//!      auto-resumes with no human in the loop (pre-P1.14 behavior).
//!
//! "The workspace" is the feature's, not the project clone's — see
//! [`crate::domain::workspace_fingerprint`]. So the mutations below are
//! the ones that reach a feature: its branch checked out and edited, its
//! branch moved by a commit. The control includes another feature moving
//! the shared clone, which must not park this one.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

const ARTIFACT_PATH: &str = "artifacts/resume-report.md";
const REPO_PATH: &str = "demeteo/resume-fingerprint";
const PROVIDER_ID: &str = "resume-fp-provider";

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

fn minimal_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Resume Fingerprint",
        "description": "Single deterministic agent step for the P1.14 resume-guard gate.",
        "steps": [
            {
                "id": "s-report",
                "kind": "agent",
                "title": "Produce report",
                "agent_kind": "stub",
                "prompt_template": format!(
                    "Produce the resume report.\n\n\
                     Feature description: {{{{feature_description}}}}\n\n\
                     @stub-write {ARTIFACT_PATH}\n"
                ),
                "capability": "artifacts",
                "allow_shell": true,
                "artifacts": [
                    {
                        "name": "resume-report",
                        "capture": { "kind": "last_write_to", "path": ARTIFACT_PATH },
                        "mode": "full"
                    }
                ],
                "on_failure": null,
                "max_iterations": 1
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
    std::fs::write(dir.join("README.md"), "# resume fixture\n").expect("seed README");
    git(&dir, &["add", "-A"]);
    git(&dir, &["commit", "-m", "seed"]);
    dir
}

async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> String {
    const MAX_WAIT: Duration = Duration::from_secs(60);
    let started = Instant::now();
    loop {
        let feature = ctx
            .features
            .get(feature_id)
            .expect("feature read")
            .expect("feature exists");
        if matches!(
            feature.status.as_str(),
            "completed" | "awaiting_mr" | "failed" | "interrupted"
        ) {
            return feature.status;
        }
        assert!(
            started.elapsed() <= MAX_WAIT,
            "feature did not settle; last status {}",
            feature.status
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
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

/// Everything life 2 needs from [`run_then_forge_crash`].
struct Crashed {
    app_data_dir: PathBuf,
    feature_id: FeatureId,
    step_exec_id: String,
    repo_dir: PathBuf,
    branch: String,
}

/// Life 1 + the forged crash.
async fn run_then_forge_crash(tag: &str) -> Crashed {
    run_then_forge_crash_recording(tag, |fp| fp).await
}

/// [`run_then_forge_crash`], with `record` choosing what the forged attempt
/// row stores given the fingerprint probed at "node start".
async fn run_then_forge_crash_recording(
    tag: &str,
    record: impl FnOnce(String) -> String,
) -> Crashed {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-resume-fp-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");
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
            name: format!("resume-fp-{tag}"),
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
        workflows::create_from_json(&ctx.workflows, &minimal_workflow()).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(FeatureLaunch {
            project_id: project.id.0.clone(),
            workflow_id: workflow_id.0.clone(),
            title: "Resume Feature".to_string(),
            description: "Produce a deterministic resume report.".to_string(),
            agent_kind: Some("stub".to_string()),
            ..Default::default()
        })
        .await
        .expect("feature_start");
    let status = poll_terminal(&ctx, &feature.id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "life 1 must succeed; got {status}"
    );

    let branch = ctx
        .features
        .get(&feature.id)
        .expect("feature read")
        .expect("feature exists")
        .resolved_branch
        .expect("a V41+ feature records its branch");

    let step = ctx
        .features
        .steps_for_feature(&feature.id)
        .expect("steps")
        .into_iter()
        .find(|s| s.step_id.0 == "s-report")
        .expect("step row");

    // Forge the exact state a driver killed mid-step leaves behind: step
    // and feature `running`, plus an open attempt row whose fingerprint
    // records the (clean) workspace this "attempt" started from.
    let fp = crate::adapters::step_executor::setup::workspace_fingerprint(
        &*ctx.exec,
        "local",
        &repo_dir.to_string_lossy(),
        &branch,
    )
    .await
    .expect("probe fingerprint");
    assert!(
        fp.ends_with(":clean"),
        "no checkout holds the branch yet: {fp}"
    );
    ctx.features
        .attempt_open(&step.id, paths::now_ms(), Some(&record(fp)))
        .expect("forge open attempt");
    ctx.features
        .step_update(
            &step.id,
            &StepExecutionPatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("forge step running");
    ctx.features
        .update(
            &feature.id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("forge feature running");

    Crashed {
        app_data_dir: tmp,
        feature_id: feature.id.clone(),
        step_exec_id: step.id.0.clone(),
        repo_dir,
        branch,
    }
}

/// Life 2 must hold the step at the synthetic gate; approval — and only
/// approval — re-runs it to success.
async fn assert_parks_until_approved(crashed: &Crashed) {
    let ctx2 = ctx_for(&crashed.app_data_dir);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let step = ctx2
        .features
        .step_get(&crate::domain::ids::StepExecutionId::from(
            crashed.step_exec_id.clone(),
        ))
        .expect("step read")
        .expect("step exists");
    assert_eq!(
        step.status, "interrupted",
        "a mismatched workspace must hold the step at the synthetic gate, not re-run it"
    );
    let feature = ctx2
        .features
        .get(&crashed.feature_id)
        .expect("feature read")
        .expect("feature exists");
    assert_eq!(
        feature.status, "awaiting_gate",
        "the feature must be parked awaiting the synthetic gate"
    );

    ctx2.presenter
        .gate_decide(&crashed.step_exec_id, "approve", None)
        .await
        .expect("approve synthetic gate");
    let status = poll_terminal(&ctx2, &crashed.feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "approval must resume the run to success; got {status}"
    );
}

/// The P1.14 exit test: a human checked the feature branch out in the
/// project clone — what a terminal opened on a pipeline does — and left an
/// edit there. The run parks; the edit survives the re-run's merge-back.
#[tokio::test]
async fn edited_feature_checkout_parks_at_synthetic_gate_until_approved() {
    let crashed = run_then_forge_crash("edited").await;
    git(&crashed.repo_dir, &["checkout", &crashed.branch]);
    std::fs::write(
        crashed.repo_dir.join("meddled.txt"),
        "changed while stopped\n",
    )
    .expect("mutate worktree");

    assert_parks_until_approved(&crashed).await;
    assert_eq!(
        std::fs::read_to_string(crashed.repo_dir.join("meddled.txt")).ok(),
        Some("changed while stopped\n".to_string()),
        "merging the re-run into a human's checkout must not discard their edit"
    );

    let _ = std::fs::remove_dir_all(&crashed.app_data_dir);
}

/// Work landed on the feature branch while the run was stopped — a
/// sequence prefix, or a human's commit — with nothing left uncommitted.
#[tokio::test]
async fn moved_feature_branch_parks_at_synthetic_gate_until_approved() {
    let crashed = run_then_forge_crash("moved").await;
    let out = std::process::Command::new("git")
        .args(["rev-parse", &crashed.branch])
        .current_dir(&crashed.repo_dir)
        .output()
        .expect("rev-parse");
    let tip = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let landed = std::process::Command::new("git")
        .args([
            "commit-tree",
            "-p",
            &tip,
            "-m",
            "landed while stopped",
            &format!("{tip}^{{tree}}"),
        ])
        .current_dir(&crashed.repo_dir)
        .output()
        .expect("commit-tree");
    let landed = String::from_utf8_lossy(&landed.stdout).trim().to_string();
    git(
        &crashed.repo_dir,
        &[
            "update-ref",
            &format!("refs/heads/{}", crashed.branch),
            &landed,
        ],
    );

    assert_parks_until_approved(&crashed).await;

    let _ = std::fs::remove_dir_all(&crashed.app_data_dir);
}

/// Control: nothing reached the feature, so the run auto-resumes with no
/// human in the loop — even though another feature moved the shared
/// project clone underneath it, which is not this feature's workspace.
#[tokio::test]
async fn untouched_feature_auto_resumes_while_the_shared_clone_moves() {
    let crashed = run_then_forge_crash("clean").await;
    git(
        &crashed.repo_dir,
        &["checkout", "-b", "demeteo/features/someone-else"],
    );
    std::fs::write(crashed.repo_dir.join("other.txt"), "another feature\n").expect("write");
    git(&crashed.repo_dir, &["add", "-A"]);
    git(
        &crashed.repo_dir,
        &["commit", "-m", "another feature's work"],
    );
    std::fs::write(crashed.repo_dir.join("scratch.txt"), "uncommitted\n").expect("write");

    let ctx2 = ctx_for(&crashed.app_data_dir);
    let status = poll_terminal(&ctx2, &crashed.feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "a feature nothing touched must auto-resume; got {status}"
    );

    let _ = std::fs::remove_dir_all(&crashed.app_data_dir);
}

/// A row recorded before the fingerprint meant the feature's workspace
/// holds `HEAD` of the shared clone — a different quantity. Comparing it
/// would park every feature interrupted across the upgrade, so it reads as
/// unknown and the run auto-resumes.
#[tokio::test]
async fn a_fingerprint_from_the_old_scheme_auto_resumes() {
    let crashed = run_then_forge_crash_recording("legacy", |_| {
        "0123456789abcdef0123456789abcdef01234567:clean".to_string()
    })
    .await;

    let ctx2 = ctx_for(&crashed.app_data_dir);
    let status = poll_terminal(&ctx2, &crashed.feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "an old-scheme fingerprint must not park the run; got {status}"
    );

    let _ = std::fs::remove_dir_all(&crashed.app_data_dir);
}
