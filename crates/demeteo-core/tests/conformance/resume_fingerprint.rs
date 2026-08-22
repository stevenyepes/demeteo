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

/// Life 1 + the forged crash. Returns everything life 2 needs:
/// `(app_data_dir, feature_id, step_execution_id, repo_dir)`.
async fn run_then_forge_crash(tag: &str) -> (PathBuf, FeatureId, String, PathBuf) {
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

    // Settle the workspace to a known-clean state, so the forged attempt's
    // fingerprint is reproducible on resume (the fingerprint is HEAD + a
    // dirty bit — a mutation on an already-dirty tree would be invisible).
    git(&repo_dir, &["add", "-A"]);
    git(&repo_dir, &["commit", "-m", "settle", "--allow-empty"]);

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
    )
    .await
    .expect("probe fingerprint");
    assert!(fp.ends_with(":clean"), "settled tree must be clean: {fp}");
    ctx.features
        .attempt_open(&step.id, paths::now_ms(), Some(&fp))
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

    (tmp, feature.id.clone(), step.id.0.clone(), repo_dir)
}

/// The P1.14 exit test: a workspace mutated between crash and resume
/// parks at the synthetic gate; approval — and only approval — re-runs.
#[tokio::test]
async fn mutated_workspace_parks_at_synthetic_gate_until_approved() {
    let (tmp, feature_id, step_exec_id, repo_dir) = run_then_forge_crash("mutated").await;

    // The between-crash-and-resume mutation: a human edited the worktree.
    std::fs::write(repo_dir.join("meddled.txt"), "changed while stopped\n")
        .expect("mutate worktree");

    // Life 2: watchdog marks the step interrupted + surfaces the
    // synthetic gate; the auto-armed driver must PARK, not re-execute.
    let ctx2 = ctx_for(&tmp);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let step = ctx2
        .features
        .step_get(&crate::domain::ids::StepExecutionId::from(
            step_exec_id.clone(),
        ))
        .expect("step read")
        .expect("step exists");
    assert_eq!(
        step.status, "interrupted",
        "a mismatched workspace must hold the step at the synthetic gate, not re-run it"
    );
    let feature = ctx2
        .features
        .get(&feature_id)
        .expect("feature read")
        .expect("feature exists");
    assert_eq!(
        feature.status, "awaiting_gate",
        "the feature must be parked awaiting the synthetic gate"
    );

    // The human blesses the changed workspace → the node re-runs and the
    // feature completes.
    ctx2.presenter
        .gate_decide(&step_exec_id, "approve", None)
        .await
        .expect("approve synthetic gate");
    let status = poll_terminal(&ctx2, &feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "approval must resume the run to success; got {status}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Control: an untouched workspace matches the recorded fingerprint and
/// auto-resumes with no human in the loop — the guard only ever bites on
/// a real mismatch.
#[tokio::test]
async fn untouched_workspace_auto_resumes_without_gating() {
    let (tmp, feature_id, _step_exec_id, _repo_dir) = run_then_forge_crash("clean").await;

    let ctx2 = ctx_for(&tmp);
    let status = poll_terminal(&ctx2, &feature_id).await;
    assert!(
        matches!(status.as_str(), "completed" | "awaiting_mr"),
        "a matching fingerprint must auto-resume; got {status}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
