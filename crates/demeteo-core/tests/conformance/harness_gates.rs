//! Multi-harness gating driver-integration fixtures (HB5,
//! `docs/HARNESS_BASELINE.md`).
//!
//! The resolution chain and the per-harness deadline are pure decisions and are
//! unit-tested next to [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses)
//! in `domain/verifier.rs`. What those cannot cover is the wiring — that the
//! engine actually
//!
//! 1. runs **every** resolved harness, in declared order, **even after one
//!    fails**, and reports each failing gate by name; and
//! 2. reaches the project's selected validation gates at all from a workflow
//!    that declares no harness of its own — which is every shipped starter, and
//!    is the whole reason tier 2 exists.
//!
//! Both legs run a **real shell** through `ExecutionMode::LocalOnly` (only the
//! *agent* is stubbed), because the thing under test is what the orchestrator
//! chose to execute, and a scripted exec double would answer for that choice
//! rather than obey it. Each harness echoes a unique marker, so "did this gate
//! run" is answerable from the step's own error message.
//!
//! Leg 2 also exercises the persistence of the gate selection end to end: it is
//! written through `save_settings` and read back by the driver, so a selection
//! that failed to round-trip through the `harnesses` column would fail here.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProjectId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/harness-gates";
const PROVIDER_ID: &str = "harness-gates-provider";

/// A gate that always fails, announcing itself first. Deterministic and
/// byte-stable, so nothing here perturbs a fingerprint.
fn failing(marker: &str) -> String {
    format!("echo '{marker}'; exit 1")
}

/// A gate that always passes, announcing itself.
fn passing(marker: &str) -> String {
    format!("echo '{marker}'; exit 0")
}

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// One agent step carrying a `verifier`. `harness` is spliced in verbatim so a
/// leg can declare the plural field, the singular one, or nothing at all — the
/// three shapes the chain distinguishes. No `on_failure`: a red harness should
/// end the run on the first attempt, keeping the fixture to one dispatch.
fn gating_workflow(harness: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "name": "Harness Gates Conformance",
        "description": "One agent step gated on several harnesses.",
        "steps": [
            {
                "id": "s-validate",
                "kind": "agent",
                "title": "Validate",
                "agent_kind": "stub",
                "prompt_template": "Validate the change. {{feature_description}}\n",
                "capability": "artifacts",
                "allow_shell": true,
                "verifier": {
                    "instructions": "Return the harness verdict.",
                    "harness_names": harness,
                    "verdict_key": "verdict"
                },
                "max_iterations": 1
            }
        ]
    })
}

/// Point the project's harness config at deterministic marker commands.
/// A fresh project has no persisted settings row (defaults are applied lazily),
/// so this builds one from the engine default.
fn set_harnesses(
    ctx: &AppContext,
    project_id: &ProjectId,
    harnesses: &[(&str, String)],
    gates: Option<&[&str]>,
    test_command: Option<String>,
) {
    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project_id.clone();
    settings.worktree_strategy.test_command = test_command;
    settings.worktree_strategy.harnesses = Some(
        harnesses
            .iter()
            .map(|(n, c)| (n.to_string(), c.clone()))
            .collect(),
    );
    settings.worktree_strategy.validation_gates =
        gates.map(|g| g.iter().map(|s| s.to_string()).collect());
    ctx.projects.save_settings(settings).expect("save settings");
}

/// Seed a real local git repo at the project's expected repo dir so
/// `bootstrap_project` skips its (network) clone — the same "already cloned"
/// shortcut every offline path relies on (mirrors the triage gate).
fn init_local_repo(workspace_dir: &Path, project_id: &str, repo_path: &str) {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, repo_path);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(&dir)
            .output()
            .expect("run git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# harness gates fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

/// Drive a freshly-started feature to a terminal state and return the validate
/// step's final error message.
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
            return ctx
                .features
                .steps_for_feature(feature_id)
                .unwrap_or_default()
                .into_iter()
                .find(|s| s.step_id.0 == "s-validate")
                .and_then(|s| s.error_message)
                .unwrap_or_default();
        }
        if started.elapsed() > MAX_WAIT {
            panic!(
                "feature {} did not settle in {:?}; last status {}",
                feature_id.as_str(),
                MAX_WAIT,
                feature.status
            );
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Register provider, create the project with the given harness config, seed the
/// repo, bootstrap, ingest the workflow and drive one feature to a terminal
/// state on a locally-executing engine. Returns the validate step's error.
async fn run_gate_leg(
    tag: &str,
    declared: serde_json::Value,
    harnesses: &[(&str, String)],
    gates: Option<&[&str]>,
    test_command: Option<String>,
) -> String {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-harness-gates-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");

    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );

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
            name: "harness-gates".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    set_harnesses(&ctx, &project.id, harnesses, gates, test_command);
    init_local_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id = workflows::create_from_json(&ctx.workflows, &gating_workflow(declared))
        .expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Gates Feature",
            "Exercise HB5 multi-harness gating.",
            Some("stub"),
            None,
            None,
            None,
            Some(1),
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start");

    let err = poll_terminal(&ctx, &feature.id).await;
    let _ = std::fs::remove_dir_all(&tmp);
    err
}

/// The headline HB5 fixture: two declared gates, the **first one red**, and the
/// second still runs. Stopping at the first failure would hand the implementer
/// half the story, which turns one wasted rework cycle into two — the cost this
/// whole document exists to avoid. The failure must also name *which* gate went
/// red, per gate, or a two-gate step is no more attributable than the
/// `&&`-chained command it replaced.
#[tokio::test]
async fn a_failing_first_gate_does_not_stop_the_second_and_both_are_named() {
    let err = run_gate_leg(
        "both",
        serde_json::json!(["lint", "unit"]),
        &[
            ("lint", failing("LINT-GATE-RAN")),
            ("unit", failing("UNIT-GATE-RAN")),
        ],
        None,
        // Tier 3 must never be consulted while tier 1 declares gates.
        Some(failing("TEST-COMMAND-RAN")),
    )
    .await;

    assert!(
        err.contains("LINT-GATE-RAN"),
        "the first declared gate must run; got: {err}"
    );
    assert!(
        err.contains("UNIT-GATE-RAN"),
        "the second gate must run even though the first already failed; got: {err}"
    );
    assert!(
        err.contains("'lint'") && err.contains("'unit'"),
        "each failure must be attributed to its gate by name; got: {err}"
    );
    assert!(
        !err.contains("TEST-COMMAND-RAN"),
        "an explicit declaration must not also drag in the project's test_command; got: {err}"
    );
}

/// Tier 2, end to end: a workflow that declares **no** harness — the shape all
/// seven starters ship — is gated by the project's selected validation gates,
/// not by its `test_command`. Before this, the `harnesses` map was dead config:
/// a user could add `lint → npm run lint`, see it accepted, and nothing would
/// ever run it short of forking a starter.
///
/// This is also the round-trip test for the selection's storage: it is written
/// through `save_settings` and read back by the driver.
#[tokio::test]
async fn a_starter_shaped_workflow_is_gated_by_the_projects_selected_gates() {
    let err = run_gate_leg(
        "tier2",
        // Exactly what every starter declares.
        serde_json::json!(null),
        &[
            ("lint", passing("LINT-GATE-RAN")),
            ("integration", failing("INTEGRATION-GATE-RAN")),
        ],
        Some(&["lint", "integration"]),
        Some(failing("TEST-COMMAND-RAN")),
    )
    .await;

    assert!(
        err.contains("INTEGRATION-GATE-RAN"),
        "the selected gates must run for a workflow that declares none; got: {err}"
    );
    assert!(
        !err.contains("TEST-COMMAND-RAN"),
        "a selection must replace the test_command fallback, not sit beside it; got: {err}"
    );
    assert!(
        err.contains("'integration'"),
        "the failing gate must be named; got: {err}"
    );
}
