//! Baseline-measurement driver-integration fixtures (HB2b / P4.2a,
//! `docs/HARNESS_BASELINE.md`).
//!
//! The two decisions this feature turns on are pure and unit-tested where they
//! live: *whether* to measure a fallback is
//! [`fallback_baseline_needed`](crate::domain::harness_baseline::fallback_baseline_needed)
//! in `domain/`, and *what a gate said* is
//! [`measure_gates`](crate::adapters::step_executor::baseline::measure_gates)
//! against a scripted port. What neither can cover is the wiring, and the
//! wiring is where a baseline goes quietly wrong:
//!
//! 1. the in-graph node records the sha it **actually measured** — the field
//!    most easily assumed and most expensive to have assumed;
//! 2. the fallback fires on validate's **failure** path and *only* there, so a
//!    green run pays nothing;
//! 3. it does **not** re-measure when a record already covers the base, which
//!    is the entire reason it persists what it measured;
//! 4. a fallback that cannot produce a measurement leaves the verdict exactly
//!    as it is today — a baseline mechanism may withhold an improvement, never
//!    invent a failure;
//! 5. the detached worktree is torn down on the success *and* the failure
//!    path.
//!
//! Every leg runs a **real shell and real git** through
//! `ExecutionMode::LocalOnly` (only the *agent* is stubbed): the subject is
//! what the orchestrator chose to run against a real repository, and a scripted
//! exec double would answer for that choice rather than obey it.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::harness_baseline::{BaselineProducer, HarnessBaseline};
use crate::domain::ids::{FeatureId, ProjectId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/harness-baseline";
const PROVIDER_ID: &str = "harness-baseline-provider";

/// A gate that always fails, announcing itself first. Byte-stable, so nothing
/// here perturbs a fingerprint between the baseline run and the live one.
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

/// The `baseline-harness` node as the starters now ship it: a `command` node
/// that declares no command at all, because the commands it runs come from the
/// project.
fn baseline_node() -> serde_json::Value {
    serde_json::json!({
        "id": "s-baseline-harness",
        "kind": "command",
        "title": "Measure Harness Baseline",
        "capability": "read_only",
        "measure_baseline": true,
        "idempotent": true,
        "max_iterations": 1
    })
}

/// One agent step gated on the project's harness — the shape every starter's
/// validate step has. No `on_failure`: a red harness should end the run on the
/// first attempt, keeping each fixture to one dispatch.
fn validate_node() -> serde_json::Value {
    serde_json::json!({
        "id": "s-validate",
        "kind": "agent",
        "title": "Validate",
        "agent_kind": "stub",
        // `@stub-verdict` matters now that HB2c can let a red-but-pre-existing
        // harness through to the agent turn: without it the turn ends with no
        // verdict object and the step fails as *infrastructure*, which would
        // mask whichever outcome the leg is actually asserting.
        "prompt_template": "Validate the change. {{feature_description}}\n@stub-verdict verdict\n",
        "capability": "artifacts",
        "allow_shell": true,
        "verifier": {
            "instructions": "Return the harness verdict.",
            "verdict_key": "verdict"
        },
        "max_iterations": 1
    })
}

fn workflow(name: &str, steps: Vec<serde_json::Value>) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": "Harness baseline conformance fixture.",
        "steps": steps,
    })
}

/// Point the project's harness config at a deterministic marker command. A
/// fresh project has no persisted settings row (defaults are applied lazily),
/// so this builds one from the engine default.
fn set_project(
    ctx: &AppContext,
    project_id: &ProjectId,
    test_command: Option<String>,
    prepare_command: Option<String>,
) {
    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project_id.clone();
    settings.worktree_strategy.test_command = test_command;
    settings.worktree_strategy.prepare_command = prepare_command;
    ctx.projects.save_settings(settings).expect("save settings");
}

/// Seed a real local git repo at the project's expected repo dir so
/// `bootstrap_project` skips its (network) clone — the same "already cloned"
/// shortcut every offline path relies on. Returns the seed commit's sha, which
/// is the commit every producer in this file should end up measuring.
fn init_local_repo(workspace_dir: &Path, project_id: &str, repo_path: &str) -> String {
    let dir = paths::repo_target_dir_local(workspace_dir, project_id, repo_path);
    std::fs::create_dir_all(&dir).expect("create repo dir");
    let git = |args: &[&str]| -> String {
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# harness baseline fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
    git(&["rev-parse", "HEAD"])
}

/// One step's terminal row, reduced to what these fixtures assert on.
#[derive(Debug, Clone, Default)]
struct StepOutcomeRow {
    status: String,
    error: Option<String>,
}

/// Drive a freshly-started feature to a terminal state and hand back the
/// baseline record plus each step's terminal row.
async fn poll_terminal(
    ctx: &AppContext,
    feature_id: &FeatureId,
) -> (Option<HarnessBaseline>, Vec<(String, StepOutcomeRow)>) {
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
            let steps = ctx
                .features
                .steps_for_feature(feature_id)
                .unwrap_or_default()
                .into_iter()
                .map(|s| {
                    (
                        s.step_id.0,
                        StepOutcomeRow {
                            status: s.status,
                            error: s.error_message,
                        },
                    )
                })
                .collect();
            return (feature.harness_baseline, steps);
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

struct LegOutcome {
    baseline: Option<HarnessBaseline>,
    steps: Vec<(String, StepOutcomeRow)>,
    /// The repo's `main` tip at the moment the feature started — the commit
    /// every producer here should be measuring.
    base_sha: String,
    /// Every leftover `<repo>_wt_*` directory. Must be empty: worktrees are
    /// disposable by construction and a run that leaves one behind leaks one
    /// checkout per attempt.
    leftover_worktrees: Vec<String>,
}

/// Register provider, create the project, seed the repo, bootstrap, ingest the
/// workflow, and drive one feature to a terminal state on a locally-executing
/// engine.
async fn run_leg(
    tag: &str,
    steps: Vec<serde_json::Value>,
    test_command: Option<String>,
    prepare_command: Option<String>,
) -> LegOutcome {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-harness-baseline-{tag}-{}",
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
            name: "harness-baseline".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    let base_sha = init_local_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    set_project(&ctx, &project.id, test_command, prepare_command);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id = workflows::create_from_json(&ctx.workflows, &workflow(tag, steps))
        .expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Baseline Feature",
            "Exercise HB2b baseline measurement.",
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

    let (baseline, steps) = poll_terminal(&ctx, &feature.id).await;

    let repo_dir = paths::repo_target_dir_local(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    let leftover_worktrees = leftover_worktrees(&repo_dir);

    let _ = std::fs::remove_dir_all(&tmp);
    LegOutcome {
        baseline,
        steps,
        base_sha,
        leftover_worktrees,
    }
}

impl LegOutcome {
    fn step(&self, id: &str) -> StepOutcomeRow {
        self.steps
            .iter()
            .find(|(step_id, _)| step_id == id)
            .map(|(_, row)| row.clone())
            .unwrap_or_else(|| panic!("no step row for '{id}' in {:?}", self.steps))
    }

    fn validate_error(&self) -> String {
        self.step("s-validate").error.unwrap_or_default()
    }
}

/// Sibling directories named `<repo>_wt_*` — every linked worktree the run
/// created and did not clean up.
fn leftover_worktrees(repo_dir: &Path) -> Vec<String> {
    let (Some(parent), Some(name)) = (repo_dir.parent(), repo_dir.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}_wt_", name.to_string_lossy());
    std::fs::read_dir(parent)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| n.starts_with(&prefix))
                .collect()
        })
        .unwrap_or_default()
}

// ── Producer 1: the in-graph node ────────────────────────────────────────────

/// The headline P4.2a leg. The node runs the project's harness at zero token
/// cost and records what it said **against the sha it measured** — not against
/// an assumed one. `covers()` exists so a baseline taken from the wrong
/// position in the graph is detectable rather than silently trusted, and it can
/// only do that if the sha is real.
#[tokio::test]
async fn the_node_records_the_sha_it_actually_measured() {
    let leg = run_leg(
        "node",
        vec![baseline_node()],
        Some(passing("BASELINE-GATE-RAN")),
        None,
    )
    .await;

    let baseline = leg
        .baseline
        .clone()
        .expect("the node must write a baseline record");
    assert_eq!(
        baseline.base_sha, leg.base_sha,
        "the record must name the commit that was measured"
    );
    assert!(
        baseline.covers(&leg.base_sha),
        "a record that does not cover the run's base is not evidence about it"
    );

    let gate = baseline
        .harness("default")
        .expect("the project's test_command is measured under the default gate name");
    assert!(gate.exit_ok, "a passing gate must be recorded as passing");
    assert_eq!(gate.producer, BaselineProducer::Node);
    assert!(
        gate.output_ref.is_some(),
        "the output belongs in the artifact store, referenced from the record"
    );
    assert!(
        leg.leftover_worktrees.is_empty(),
        "the node's worktree must be torn down: {:?}",
        leg.leftover_worktrees
    );
}

/// A harness that is already red at the base is **recorded, not judged**. That
/// is the entire point of a baseline: a repository whose suite was already
/// failing is not this feature's defect, and failing the run at its very first
/// node would restate exactly the misattribution HB2 exists to remove — before
/// a single line has been written.
#[tokio::test]
async fn a_red_gate_at_the_base_is_recorded_and_the_run_continues() {
    let leg = run_leg(
        "node-red",
        vec![baseline_node()],
        Some(failing("BASELINE-GATE-RAN")),
        None,
    )
    .await;

    let gate = leg
        .baseline
        .clone()
        .expect("a red baseline is still a baseline")
        .harness("default")
        .cloned()
        .expect("the gate was measured");
    assert!(!gate.exit_ok, "the gate must be recorded as red");
    assert!(
        !gate.fingerprint.is_empty(),
        "a red gate owes a fingerprint — it is the cheap rung of HB2c's ladder"
    );

    // The load-bearing half: the step itself must not fail. Failing here would
    // block every run against an already-red repository at its very first node,
    // which is the misattribution this whole subsystem exists to remove.
    let step = leg.step("s-baseline-harness");
    assert_eq!(
        step.status, "completed",
        "a red gate at the base is recorded, not judged: {step:?}"
    );
    assert_eq!(step.error, None, "and it is not dressed as a failure");
    assert!(
        leg.leftover_worktrees.is_empty(),
        "the worktree must be torn down on the red path too: {:?}",
        leg.leftover_worktrees
    );
}

// ── Producer 2: the lazy fallback ────────────────────────────────────────────

/// The leg HB2b exists for: a workflow with **no baseline node** — a custom
/// workflow, or any of the five starters that did not get one — still ends up
/// with a record, because validate's failure path measured one itself.
#[tokio::test]
async fn a_red_validate_with_no_baseline_node_measures_a_fallback() {
    let leg = run_leg(
        "fallback",
        vec![validate_node()],
        Some(failing("HARNESS-RAN")),
        None,
    )
    .await;

    let baseline = leg
        .baseline
        .clone()
        .expect("the fallback must write a record when validate goes red");
    assert_eq!(
        baseline.base_sha, leg.base_sha,
        "the fallback measures the merge-base, not the feature branch tip — \
         measuring the tip would compare the work against itself"
    );
    let gate = baseline
        .harness("default")
        .expect("the red gate is measured");
    assert_eq!(gate.producer, BaselineProducer::Fallback);
    assert!(
        !gate.exit_ok,
        "this repo's harness fails at the base too, which is what makes the \
         failure pre-existing"
    );
    // What the *verdict* then does with that record is HB2c's, and it is
    // asserted in `harness_subtraction.rs`: this gate is red at the base with
    // the identical failure, so it no longer fails the step. Re-asserting it
    // here would duplicate that leg while pretending to be about the producer.
    assert!(
        !gate.fingerprint.is_empty(),
        "a red gate owes a fingerprint"
    );
    assert!(
        leg.leftover_worktrees.is_empty(),
        "the detached worktree must be torn down: {:?}",
        leg.leftover_worktrees
    );
}

/// The fallback must **never** fire on green. There is nothing to subtract
/// from, and measuring anyway would add minutes to every successful run,
/// forever, to answer a question nobody asked.
#[tokio::test]
async fn a_green_validate_measures_nothing() {
    let leg = run_leg(
        "green",
        vec![validate_node()],
        Some(passing("HARNESS-RAN")),
        None,
    )
    .await;

    assert!(
        leg.baseline.is_none(),
        "a green run must not pay for a baseline: {:?}",
        leg.baseline
    );
    assert!(leg.leftover_worktrees.is_empty());
}

/// With the node ahead of it, validate's failure path must find the record
/// already covering the base and **not re-measure**. Provenance is what makes
/// this observable: the gate stays stamped `node`, so a fallback that ran
/// anyway would show up as `fallback`.
///
/// This is also the cross-producer agreement leg. The node measures its
/// worktree's HEAD and the fallback measures `git merge-base`; if those two
/// ever disagreed about what "the base" is, the record would not cover and the
/// fallback would re-measure on every single validate failure.
#[tokio::test]
async fn a_covering_record_is_not_re_measured_by_the_fallback() {
    let leg = run_leg(
        "cached",
        vec![baseline_node(), validate_node()],
        Some(failing("HARNESS-RAN")),
        None,
    )
    .await;

    let baseline = leg.baseline.clone().expect("the node wrote a record");
    let gate = baseline
        .harness("default")
        .expect("the gate is in the record");
    assert_eq!(
        gate.producer,
        BaselineProducer::Node,
        "a covering record must satisfy validate's failure path outright — \
         re-measuring would pay for the same answer twice per attempt"
    );
    assert_eq!(baseline.base_sha, leg.base_sha);
    assert!(leg.leftover_worktrees.is_empty());
}

/// A fallback that cannot produce a measurement must leave everything exactly
/// as it is today.
///
/// The condition is real rather than contrived: the project's
/// `prepare_command` succeeds in validate's own worktree — which is checked out
/// on a branch — and fails in the baseline worktree, which is **detached** by
/// construction. That is the shape of every environment-sensitive prepare
/// step, and it is the one case where measuring anyway would be actively
/// harmful: a suite run without its install step fails for reasons that have
/// nothing to do with the base commit, so recording those gates as red-at-base
/// would excuse a real regression later.
///
/// So: no record at all, and the run reaches the same verdict, naming the same
/// gate, that it reached before this feature existed. A broken baseline
/// mechanism may withhold an improvement; it may never invent a failure.
#[tokio::test]
async fn a_fallback_that_cannot_measure_leaves_the_verdict_untouched() {
    let leg = run_leg(
        "unmeasurable",
        vec![validate_node()],
        Some(failing("HARNESS-RAN")),
        // Succeeds on a branch, fails on a detached HEAD.
        Some("git symbolic-ref -q HEAD >/dev/null".to_string()),
    )
    .await;

    assert!(
        leg.baseline.is_none(),
        "a measurement taken without its prepare step is not evidence about \
         the base commit, so none may be recorded: {:?}",
        leg.baseline
    );
    assert!(
        leg.validate_error().contains("HARNESS-RAN"),
        "the verdict must be the harness's own, unchanged: {}",
        leg.validate_error()
    );
    assert!(
        leg.leftover_worktrees.is_empty(),
        "the worktree must be torn down even when the measurement failed: {:?}",
        leg.leftover_worktrees
    );
}
