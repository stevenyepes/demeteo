//! Bootstrap harness-preflight gate (HB1, `docs/HARNESS_BASELINE.md`).
//!
//! The preflight's decision logic is unit-tested next to its definition in
//! `adapters/step_executor/preflight.rs`, against a scripted `ExecutionPort`
//! double. What those tests cannot cover is the wiring, which is the entire
//! user-facing claim:
//!
//! 1. a project whose `test_command` names a binary the login shell cannot find
//!    **never starts** — the feature reaches a terminal `failed` and, crucially,
//!    **no step rows are ever seeded**, so nothing is spent;
//! 2. the failure is reported on the bootstrap stepper as a `harness_preflight`
//!    phase carrying the missing binary's name, not as a mystery;
//! 3. a project whose commands *do* resolve is unaffected — the phase passes and
//!    the pipeline proceeds exactly as before.
//!
//! Point 3 is the one that needs a real shell rather than a double, because the
//! risk this feature carries is a **false positive**: a preflight that wrongly
//! blocks a working project is worse than no preflight at all. So this runs
//! `ExecutionMode::LocalOnly` — a real `LocalSubprocessAdapter`, a real
//! `command -v` under a real interactive login shell. Only the *agent* is
//! stubbed (C5's `StubRuntime`).
//!
//! Both legs share one workflow and differ only in the project's configured
//! `test_command`.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/preflight";
const PROVIDER_ID: &str = "preflight-provider";

/// A binary name no machine will have on `PATH`. Deliberately not a plausible
/// tool: a test that fails because CI happens to have the thing installed
/// would be worse than no test.
const ABSENT_BINARY: &str = "demeteo-definitely-not-installed-xyz";

/// Captures the bootstrap phases the run emitted, in order, so a test can
/// assert both *that* the preflight spoke and *what* it said.
#[derive(Default)]
struct PhaseRecorder {
    phases: Mutex<Vec<(String, String, Option<String>)>>,
}

impl PhaseRecorder {
    fn preflight(&self) -> Vec<(String, String, Option<String>)> {
        self.phases
            .lock()
            .unwrap()
            .iter()
            .filter(|(id, _, _)| id == "harness_preflight")
            .cloned()
            .collect()
    }

    fn saw_phase(&self, id: &str) -> bool {
        self.phases.lock().unwrap().iter().any(|(p, _, _)| p == id)
    }
}

struct RecordingNotif(Arc<PhaseRecorder>);

impl NotificationPort for RecordingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        if let DomainEvent::BootstrapProgress {
            phase,
            status,
            detail,
            ..
        } = event
        {
            self.0
                .phases
                .lock()
                .unwrap()
                .push((phase.clone(), status.clone(), detail.clone()));
        }
        Ok(())
    }
}

/// One agent step. It never runs in the blocking leg — that is the point — and
/// in the passing leg it only has to complete for the run to have proceeded.
fn preflight_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Preflight Conformance",
        "description": "One agent step, used to observe whether the run started at all.",
        "steps": [
            {
                "id": "s-work",
                "kind": "agent",
                "title": "Work",
                "agent_kind": "stub",
                "prompt_template": "Do the thing. {{feature_description}}\n",
                "capability": "artifacts",
                "allow_shell": true,
                "on_failure": null,
                "max_iterations": 1
            }
        ]
    })
}

/// Seed a real local git repo so `bootstrap_project` skips its network clone.
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
    std::fs::write(dir.join("README.md"), "# preflight conformance fixture\n")
        .expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
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
            "feature {} did not settle in {:?}; last status {}",
            feature_id.as_str(),
            MAX_WAIT,
            feature.status
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Run one leg with `test_command` set to `harness`, returning the terminal
/// feature status, the recorded phases, and how many step rows were seeded.
async fn run_leg(label: &str, harness: &str) -> (String, Arc<PhaseRecorder>, usize) {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let recorder = Arc::new(PhaseRecorder::default());
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-preflight-{label}-{}",
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
        Arc::new(RecordingNotif(recorder.clone())),
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
            name: "preflight".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project.id.clone();
    settings.worktree_strategy.test_command = Some(harness.to_string());
    ctx.projects.save_settings(settings).expect("save settings");

    init_local_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id = workflows::create_from_json(&ctx.workflows, &preflight_workflow())
        .expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Preflight Feature",
            "Exercise the HB1 bootstrap preflight gate.",
            Some("stub"),
            None,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start");

    let status = poll_terminal(&ctx, &feature.id).await;
    let step_count = ctx
        .features
        .steps_for_feature(&feature.id)
        .expect("list steps")
        .len();
    let _ = std::fs::remove_dir_all(&tmp);
    (status, recorder, step_count)
}

/// The headline fixture: an unresolvable binary stops the run at launch.
///
/// Today — without the preflight — this same project runs research, tickets,
/// spec and the whole implement sequence before `run_harness_first` discovers
/// the binary is missing, and then reports it wearing the costume of a failed
/// feature. The assertion that no step rows exist is the one that says "nothing
/// was spent"; the terminal status alone would not distinguish this from a run
/// that failed late.
#[tokio::test]
async fn an_unresolvable_binary_blocks_the_launch_before_any_step_is_seeded() {
    let (status, rec, step_count) =
        run_leg("missing", &format!("{ABSENT_BINARY} --run && echo done")).await;

    assert_eq!(status, "failed", "an unresolvable harness must not start");
    assert_eq!(
        step_count, 0,
        "the preflight runs before `registering`, so a blocked launch must leave \
         no step rows at all — otherwise the UI shows a pipeline that never ran"
    );

    let phases = rec.preflight();
    assert!(
        phases.iter().any(|(_, status, _)| status == "failed"),
        "the stepper must show the preflight failing; got {phases:?}"
    );
    let detail = phases
        .iter()
        .find(|(_, s, _)| s == "failed")
        .and_then(|(_, _, d)| d.clone())
        .expect("a failing preflight must carry a detail");
    assert!(
        detail.contains(ABSENT_BINARY),
        "the detail must name the binary the user has to install or fix; got:\n{detail}"
    );

    assert!(
        !rec.saw_phase("starting_pipeline"),
        "the pipeline must never be started; got {:?}",
        rec.phases.lock().unwrap()
    );
}

/// The false-positive guard, and the reason this leg uses a real shell: a
/// project whose commands resolve must be completely unaffected. `sh` is on
/// every machine this suite can run on, including the CI container.
#[tokio::test]
async fn a_resolvable_binary_leaves_the_run_untouched() {
    let (_status, rec, step_count) = run_leg("resolvable", "sh -c 'exit 0'").await;

    let phases = rec.preflight();
    assert!(
        phases.iter().all(|(_, s, _)| s != "failed"),
        "a resolvable command must not be flagged; got {phases:?}"
    );
    assert!(
        rec.saw_phase("starting_pipeline"),
        "the pipeline must start normally; got {:?}",
        rec.phases.lock().unwrap()
    );
    assert!(
        step_count > 0,
        "step rows must be seeded exactly as before the preflight existed"
    );
}
