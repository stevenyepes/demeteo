//! `clean` gate autonomy through the driver: a review step's turn, the store,
//! and the ship gate that reads it back.
//!
//! The policy, the contract and the store read each have unit tests of their
//! own. These are the tests that the driver calls them — replacing the gate's
//! review read with a constant passes every one of those and fails here.
//!
//! Real engine, real local `ExecutionPort`, and the deterministic
//! `StubRuntime`, as in [`launch_resolution`](super::launch_resolution).

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::harness::CapturingNotif;
use crate::adapters::agent::stub_runtime::{PROMPT_LOG, STUB_AGENT_ENV};
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::gate_autonomy::GateAutonomy;
use crate::domain::ids::ProviderId;
use crate::domain::models::{Feature, ProviderInstance, StepExecution};
use crate::paths;
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

const REVIEW_PATH: &str = "artifacts/s-review.md";

/// A review step and the ship gate it covers. The review's artifact name is
/// deliberately not its file's stem, which is how most workflows declare one.
/// `directive` is what the stub is told to write, and `marker` makes the
/// rendered prompt findable in a log every stub-driven test shares.
fn review_then_ship(marker: &str, directive: &str) -> serde_json::Value {
    serde_json::json!({
        "name": format!("Clean Review {marker}"),
        "description": "A review step and the ship gate it covers.",
        "steps": [
            {
                "id": "s-review",
                "kind": "agent",
                "title": "Review",
                "agent_kind": "stub",
                "prompt_template": format!("Review the change. {marker}\n\n{directive}\n"),
                "capability": "artifacts",
                "allow_shell": true,
                "artifacts": [{
                    "name": "review",
                    "capture": { "kind": "last_write_to", "path": REVIEW_PATH },
                    "mode": "full"
                }],
                "review_artifact": "review",
                "on_failure": null,
                "max_iterations": 1
            },
            {
                "id": "s-ship",
                "kind": "gate",
                "gate_class": "dangerous",
                "title": "Ship",
                "agent_kind": null,
                "prompt_template": "Approve to ship.",
                "on_failure": null,
                "max_iterations": null
            }
        ]
    })
}

struct Run {
    ctx: AppContext,
    notif: Arc<CapturingNotif>,
    feature: Feature,
    tmp: std::path::PathBuf,
}

impl Run {
    /// `None` until the driver has seeded the run's rows, which it does
    /// after `feature_start` has already returned.
    fn step(&self, step_id: &str) -> Option<StepExecution> {
        self.ctx
            .features
            .steps_for_feature(&self.feature.id)
            .expect("steps read")
            .into_iter()
            .find(|s| s.step_id.as_ref() == step_id)
    }

    fn status(&self, step_id: &str) -> String {
        self.step(step_id).map(|s| s.status).unwrap_or_default()
    }

    /// Poll until `settled` says the run has reached the state under test.
    async fn until(&self, what: &str, settled: impl Fn(&Self) -> bool) {
        let started = Instant::now();
        while !settled(self) {
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "the run never reached: {what}; review is {:?}, gate is {:?}",
                self.step("s-review").map(|s| (s.status, s.error_message)),
                self.status("s-ship"),
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn events(&self) -> Vec<DomainEvent> {
        self.notif.events.lock().unwrap().clone()
    }

    fn approved_by_policy(&self) -> bool {
        self.events().iter().any(|event| {
            matches!(
                event,
                DomainEvent::GateDecided {
                    auto_approved: true,
                    ..
                }
            )
        })
    }

    fn held_reasons(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter_map(|event| match event {
                DomainEvent::GateHeldByReview { reason, .. } => Some(reason),
                _ => None,
            })
            .collect()
    }

    async fn close(self) {
        let _ = self
            .ctx
            .executor
            .feature_cancel(self.feature.id.as_ref())
            .await;
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

/// Launch [`review_then_ship`] on a project set to `clean`. `committed` is a
/// review already tracked at [`REVIEW_PATH`] on the branch the run is cut
/// from — what a project that commits its step reports inherits from its
/// last feature.
async fn launch(label: &str, directive: &str, committed: Option<&str>) -> Run {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let repo_path = format!("demeteo/{label}");
    let tmp = crate::support::test_dir::scratch(&format!("demeteo-{label}"));
    let notif = Arc::new(CapturingNotif::default());
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        notif.clone(),
        tokio::runtime::Handle::current(),
    );

    ctx.app_settings
        .add_provider_instance(ProviderInstance {
            id: ProviderId::from("clean-review-provider"),
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
            name: label.to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: repo_path.clone(),
                provider_id: "clean-review-provider".to_string(),
            }],
        },
    )
    .expect("create project");

    let dir = paths::repo_target_dir_local(&ctx.workspace_dir, project.id.as_str(), &repo_path);
    std::fs::create_dir_all(dir.join("artifacts")).expect("create repo dir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run git");
        assert!(out.status.success(), "git {args:?} failed");
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# clean review fixture\n").expect("seed README");
    if let Some(review) = committed {
        std::fs::write(dir.join(REVIEW_PATH), review).expect("seed the earlier review");
    }
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let mut settings = ctx
        .projects
        .get_settings(&project.id)
        .expect("settings read")
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
    settings.project_id = project.id.clone();
    settings.gate_autonomy = GateAutonomy::Clean;
    ctx.projects.save_settings(settings).expect("save settings");

    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &review_then_ship(label, directive))
            .expect("ingest workflow");
    let feature = ctx
        .executor
        .feature_start(FeatureLaunch {
            project_id: project.id.0.clone(),
            workflow_id: workflow_id.0.clone(),
            title: format!("Clean review {label}"),
            description: "Reach the ship gate under clean autonomy.".to_string(),
            agent_kind: Some("stub".to_string()),
            ..Default::default()
        })
        .await
        .expect("feature_start");

    Run {
        ctx,
        notif,
        feature,
        tmp,
    }
}

const EARLIER_CLEAN_REVIEW: &str = "## Critical Issues\nNone.\n\n## Major Issues\nNone.\n\n\
## Minor Issues\nNone.\n\n## Verdict\nPASS\n\n\
{\"review\": {\"critical\": 0, \"major\": 0, \"minor\": 0}}\n";

#[tokio::test]
async fn a_clean_review_ships_the_gate_without_anyone_being_asked() {
    let run = launch(
        "clean-review-ships",
        &format!("@stub-review {REVIEW_PATH} 0 0 1"),
        None,
    )
    .await;

    run.until("the ship gate completing", |run| {
        run.status("s-ship") == "completed"
    })
    .await;

    let gate = run.step("s-ship").expect("the gate has a row");
    let decision = run
        .ctx
        .gates
        .latest_for_step(&gate.id)
        .expect("gate read")
        .expect("the gate has a decision row");
    assert_eq!(decision.decision.as_deref(), Some("approve"));
    assert!(decision.auto_approved);
    assert!(run.approved_by_policy());
    assert_eq!(run.held_reasons(), Vec::<String>::new());
    run.close().await;
}

#[tokio::test]
async fn a_blocking_review_parks_the_gate_and_says_why() {
    let run = launch(
        "clean-review-blocks",
        &format!("@stub-review {REVIEW_PATH} 0 1 0"),
        None,
    )
    .await;

    run.until("the ship gate being held for a person", |run| {
        !run.held_reasons().is_empty()
    })
    .await;

    assert_eq!(
        run.held_reasons(),
        vec!["the review lists critical or major issues".to_string()]
    );
    assert_eq!(run.status("s-ship"), "awaiting_gate");
    assert!(!run.approved_by_policy());
    run.close().await;
}

/// The step's own template says nothing about the format; the section the
/// agent is told to follow is the one the engine appends.
#[tokio::test]
async fn a_review_step_is_handed_the_review_format() {
    let run = launch(
        "clean-review-prompt",
        &format!("@stub-review {REVIEW_PATH} 0 0 0"),
        None,
    )
    .await;

    run.until("the review step completing", |run| {
        run.status("s-review") == "completed"
    })
    .await;

    let prompts: Vec<String> = PROMPT_LOG
        .lock()
        .unwrap()
        .iter()
        .filter(|prompt| prompt.contains("Review the change. clean-review-prompt"))
        .cloned()
        .collect();
    assert_eq!(prompts.len(), 1, "the review step ran one turn");
    assert!(prompts[0].contains("## Review Format (orchestrator contract)"));
    assert!(prompts[0].contains(&format!("Write the review to `{REVIEW_PATH}`")));
    run.close().await;
}

/// The turn writes nothing, and a passing review from an earlier feature is
/// already tracked at the path. That review is not this run's, so the step
/// has not delivered and the gate is never reached.
#[tokio::test]
async fn a_review_the_turn_did_not_write_is_not_its_review() {
    let run = launch("clean-review-stale", "", Some(EARLIER_CLEAN_REVIEW)).await;

    run.until("the review step settling", |run| {
        matches!(run.status("s-review").as_str(), "failed" | "completed")
    })
    .await;

    let review = run.step("s-review").expect("the review step has a row");
    assert_eq!(review.status, "failed");
    assert!(
        review
            .error_message
            .as_deref()
            .is_some_and(|message| message.contains(REVIEW_PATH)),
        "the failure should name the review that was not written: {:?}",
        review.error_message
    );
    assert_ne!(run.status("s-ship"), "completed");
    assert!(!run.approved_by_policy());
    run.close().await;
}
