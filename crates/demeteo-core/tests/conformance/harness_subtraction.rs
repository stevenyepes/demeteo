//! The subtraction, end to end (HB2c, `docs/HARNESS_BASELINE.md`, decision 44).
//!
//! The decision itself is pure and unit-tested where it lives
//! ([`compare_gate`](crate::domain::harness_delta::compare_gate) in `domain/`),
//! as is the wording of every block it produces. What neither can cover is that
//! a *running feature* reaches those answers — and that is where a subtraction
//! goes quietly wrong, because its failure mode is silence: a verdict that no
//! longer fires, or one that fires on evidence about the wrong commit.
//!
//! The two legs the whole task exists for:
//!
//! 1. a gate red at the base and identically red now does **not** produce a
//!    verdict failure — the run continues, and the exclusion travels into the
//!    evidence the validate turn is handed;
//! 2. a gate green at the base and red now **does**.
//!
//! And the one that keeps leg 1 from being a way to pass anything: a gate red at
//! the base **because this machine cannot run it** is indistinguishable from leg
//! 1 on every input the comparison can see, and must terminate with remediation
//! instead of being subtracted. The difference is not computed — it is the
//! classification C6's agent recorded when the baseline was measured.
//!
//! Plus the three that keep it honest: a differently-red gate is still this
//! feature's, an exclusion is named beside an attributable failure rather than
//! vanishing, and C6's classifier is not consulted about a case the measurement
//! already answered.
//!
//! Every leg runs a **real shell and real git** through
//! `ExecutionMode::LocalOnly` (only the *agent* is stubbed). Two fixture
//! techniques recur and are worth reading once:
//!
//! * **`git symbolic-ref -q HEAD`** distinguishes the two worktrees a run has.
//!   HB2b measures the baseline in a worktree that is *detached* by
//!   construction, while the step itself runs on a branch — so a command
//!   guarded on it behaves one way at the base and another at the tip. That is
//!   how a green-then-red gate, or a differently-red one, is produced
//!   deterministically without a second commit.
//! * **`@stub-verdict` inside a gate's output** is the observation channel for
//!   "did this block reach the agent?". The stub reads its directives from the
//!   rendered prompt, so a directive that exists *only* in an excluded gate's
//!   output makes the turn's verdict proof that the exclusion was rendered. Drop
//!   the excluded block from the prompt and the turn ends with no verdict at
//!   all.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::{PROMPT_LOG, STUB_AGENT_ENV};
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::harness_baseline::HarnessBaseline;
use crate::domain::ids::{FeatureId, ProjectId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/harness-subtraction";
const PROVIDER_ID: &str = "harness-subtraction-provider";

/// Records the two terminal signals the C6 paths emit, so a leg can assert
/// which branch a run actually took.
#[derive(Default)]
struct SignalRecorder {
    environment_not_ready: Mutex<Vec<String>>,
    retry_budget_exhausted: Mutex<Vec<String>>,
}

struct RecordingNotif(Arc<SignalRecorder>);

impl NotificationPort for RecordingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        match event {
            DomainEvent::EnvironmentNotReady { reason, .. } => {
                self.0
                    .environment_not_ready
                    .lock()
                    .unwrap()
                    .push(reason.clone());
            }
            DomainEvent::RetryBudgetExhausted { reason, .. } => {
                self.0
                    .retry_budget_exhausted
                    .lock()
                    .unwrap()
                    .push(reason.clone());
            }
            _ => {}
        }
        Ok(())
    }
}

/// A gate that fails on a branch and **passes** on a detached HEAD, i.e. green
/// at the base and red at the tip: the regression row of decision 44's table.
fn regressed(marker: &str) -> String {
    format!("echo '{marker}'; git symbolic-ref -q HEAD >/dev/null || exit 0; exit 1")
}

/// A gate that fails identically wherever it runs: the pre-existing row. The
/// output is byte-stable, so the fingerprint the baseline records and the one
/// the live run computes are the same string.
fn always_red(marker: &str) -> String {
    format!("echo '{marker}'; exit 1")
}

/// A gate that is red both sides but says something *more* at the tip — new
/// failures on top of a pre-existing one.
fn red_plus(marker: &str, extra: &str) -> String {
    format!("echo '{marker}'; git symbolic-ref -q HEAD >/dev/null && echo '{extra}'; exit 1")
}

/// One agent step gated on the project's harness, with no `@stub-verdict` of
/// its own: the only way this turn can produce a verdict is if a gate's output
/// carried the directive into the prompt.
fn validate_node(on_failure: Option<&str>, max_iterations: u32) -> serde_json::Value {
    serde_json::json!({
        "id": "s-validate",
        "kind": "agent",
        "title": "Validate",
        "agent_kind": "stub",
        "prompt_template": "Validate the change. {{feature_description}}\n",
        "capability": "artifacts",
        "allow_shell": true,
        "verifier": {
            "instructions": "Return the harness verdict.",
            "verdict_key": "verdict"
        },
        "on_failure": on_failure,
        "max_iterations": max_iterations
    })
}

/// How a leg configures the project's harness.
#[derive(Default)]
struct GateConfig {
    /// `(name, command)` pairs for the project's `harnesses` map.
    harnesses: Vec<(String, String)>,
    /// Names ticked as validation gates (HB5 tier 2).
    validation_gates: Option<Vec<String>>,
    /// The project's `test_command` (HB5 tier 3).
    test_command: Option<String>,
}

fn set_project(ctx: &AppContext, project_id: &ProjectId, config: &GateConfig) {
    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project_id.clone();
    settings.worktree_strategy.test_command = config.test_command.as_deref().map(posix_script);
    if !config.harnesses.is_empty() {
        settings.worktree_strategy.harnesses = Some(
            config
                .harnesses
                .iter()
                .map(|(name, command)| (name.clone(), posix_script(command)))
                .collect(),
        );
    }
    settings.worktree_strategy.validation_gates = config.validation_gates.clone();
    ctx.projects.save_settings(settings).expect("save settings");
}

fn posix_script(command: &str) -> crate::domain::models::ScriptVariants {
    crate::domain::models::ScriptVariants {
        posix: Some(command.to_string()),
        powershell: None,
    }
}

/// Seed a real local git repo so `bootstrap_project` skips its network clone.
/// Returns the seed commit's sha — the commit every baseline here measures.
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
    std::fs::write(dir.join("README.md"), "# harness subtraction fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
    git(&["rev-parse", "HEAD"])
}

#[derive(Debug, Clone, Default)]
struct StepOutcomeRow {
    status: String,
    error: Option<String>,
}

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
    base_sha: String,
    signals: Arc<SignalRecorder>,
}

impl LegOutcome {
    fn validate(&self) -> StepOutcomeRow {
        self.steps
            .iter()
            .find(|(id, _)| id == "s-validate")
            .map(|(_, row)| row.clone())
            .unwrap_or_else(|| panic!("no s-validate row in {:?}", self.steps))
    }

    fn validate_error(&self) -> String {
        self.validate().error.unwrap_or_default()
    }

    fn gate(&self, name: &str) -> crate::domain::harness_baseline::HarnessBaselineRun {
        self.baseline
            .as_ref()
            .unwrap_or_else(|| panic!("no baseline record was written"))
            .harness(name)
            .unwrap_or_else(|| panic!("gate '{name}' is not in the record"))
            .clone()
    }
}

/// Every prompt the stub was handed that carries `marker`. The log is shared
/// with any other stub-driven test running concurrently, so each leg filters by
/// a string only its own fixture can produce.
fn prompts_containing(marker: &str) -> Vec<String> {
    PROMPT_LOG
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.contains(marker))
        .cloned()
        .collect()
}

async fn run_leg(tag: &str, steps: Vec<serde_json::Value>, config: GateConfig) -> LegOutcome {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-harness-subtraction-{tag}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).expect("create app data dir");

    let signals = Arc::new(SignalRecorder::default());
    let ctx = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(RecordingNotif(signals.clone())),
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
            name: "harness-subtraction".to_string(),
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
    set_project(&ctx, &project.id, &config);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id = workflows::create_from_json(
        &ctx.workflows,
        &serde_json::json!({
            "name": tag,
            "description": "HB2c subtraction conformance fixture.",
            "steps": steps,
        }),
    )
    .expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Subtraction Feature",
            "Exercise HB2c subtraction.",
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
    let _ = std::fs::remove_dir_all(&tmp);
    LegOutcome {
        baseline,
        steps,
        base_sha,
        signals,
    }
}

// ── Leg 1: red before, identically red now ───────────────────────────────────

/// **The leg the whole plan exists for.** A repository whose suite was already
/// failing is not this feature's defect, and before HB2c every such run went
/// into a rework loop over it — $14.63 and 11M tokens per cycle in the run
/// `docs/HARNESS_BASELINE.md` §1 records.
///
/// The gate is red here and red at the base with byte-identical output, so it
/// must not fail the step. That the *excluded block reached the agent* is
/// proven by the verdict existing at all: the step's own prompt carries no
/// `@stub-verdict`, so the only place the stub could have read one is the
/// excluded gate's rendered output.
#[tokio::test]
async fn a_gate_red_at_the_base_and_identically_red_now_does_not_fail_the_step() {
    let leg = run_leg(
        "pre-existing",
        vec![validate_node(None, 1)],
        GateConfig {
            // The directive is emitted *before* the non-zero exit, and it is
            // the only `@stub-verdict` anywhere in this leg.
            test_command: Some(
                "echo 'PRE-EXISTING-RED'; echo '@stub-verdict verdict'; exit 1".to_string(),
            ),
            ..Default::default()
        },
    )
    .await;

    let row = leg.validate();
    assert_eq!(
        row.status, "completed",
        "a failure that predates the feature must not be charged to it: {row:?}"
    );
    assert_eq!(row.error, None, "and it must not be dressed as a failure");

    // The exclusion is *named* in the evidence the turn was judging on. Passing
    // a red gate silently would be a subtraction the reader cannot audit — and
    // the first time it is wrong, nothing would say it happened.
    // Two turns see this gate's output, and telling them apart is itself the
    // assertion. The baseline's triage call reads it once at measurement time —
    // that is the whole cost of rung 3, paid at the head of the failure path —
    // and the validate turn reads it once as evidence. Split them by the
    // heading each carries and neither can borrow the other's count.
    let prompts = prompts_containing("PRE-EXISTING-RED");
    let triage: Vec<&String> = prompts
        .iter()
        .filter(|p| p.contains("triage classifier"))
        .collect();
    let judged: Vec<&String> = prompts
        .iter()
        .filter(|p| p.contains("Harness Results"))
        .collect();
    assert_eq!(
        triage.len(),
        1,
        "the red baseline gate is classified exactly once, at measurement time"
    );
    assert_eq!(
        judged.len(),
        1,
        "exactly one validate turn should have seen this gate"
    );
    let prompt = judged[0];
    assert!(
        prompt.contains("Excluded"),
        "the excluded gate needs a heading of its own in the prompt:\n{prompt}"
    );
    assert!(
        prompt.contains("Record each excluded gate in your report"),
        "and the report has to carry it too, or the audit trail stops at the \
         prompt:\n{prompt}"
    );

    // The subtraction rested on a real measurement of the run's own base — not
    // on the absence of one.
    let gate = leg.gate("default");
    assert!(
        !gate.exit_ok,
        "the gate was red at the base; that is what excused it"
    );
    // This leg also exercises the classifier's fail-safe end to end. The
    // baseline's red gate *was* handed to the triage agent (every red baseline
    // gate is), and the stub had no `@stub-triage` directive to answer with — so
    // it returned nothing parseable, which `parse_triage_text` reads as
    // `regression`. That must leave the gate subtractable and the run
    // untouched: a classifier that cannot answer withholds an escalation, it
    // never manufactures one.
    assert!(
        gate.environment.is_none(),
        "a triage that answered nothing must not have terminated the run: {:?}",
        gate.environment
    );
    assert_eq!(
        leg.baseline.as_ref().unwrap().base_sha,
        leg.base_sha,
        "and the record must cover this run's base commit"
    );
}

// ── Leg 1b: red before, but because the gate cannot run here ─────────────────

/// **The row leg 1 must not swallow.** Every input the comparison can see is
/// leg 1's: the same command, the same byte-identical output both sides, the
/// same red exit. What differs is *why* it was red at the base — the machine
/// cannot run the gate — and that is knowable only from the classification the
/// baseline measurement recorded.
///
/// Subtracting it would pass the step on a gate that verified nothing, which is
/// strictly worse than the misattribution HB2c set out to fix: it turns "does
/// validate hide issues?" from a question into a yes. Before decision 44 this
/// case reached C6's classifier and terminated with remediation, so a silent
/// pass here is also a regression against master.
///
/// The fixture is deliberately built so that a *wrong* answer is loud: the gate
/// output carries `@stub-verdict verdict` as well, so if the gate is excluded
/// the excluded block reaches the validate turn, the stub returns a passing
/// verdict, and the step **completes green**. There is no ambiguous middle.
///
/// It also carries `@stub-triage environment`, which is what the baseline's own
/// triage call reads — at the head of the failure path, before any rework cycle
/// has been spent. That is the cost argument in executable form: the same agent,
/// the same prompt, one cycle earlier than `should_triage` could ever reach it.
#[tokio::test]
async fn a_gate_that_could_not_run_at_the_base_terminates_rather_than_being_excluded() {
    let leg = run_leg(
        "unrunnable",
        vec![validate_node(None, 1)],
        GateConfig {
            test_command: Some(
                "echo 'UNRUNNABLE-GATE-RAN'; echo '@stub-triage environment'; \
                 echo '@stub-verdict verdict'; exit 1"
                    .to_string(),
            ),
            ..Default::default()
        },
    )
    .await;

    let signals = leg.signals.environment_not_ready.lock().unwrap().clone();
    assert_eq!(
        signals.len(),
        1,
        "a gate that proved nothing must terminate with remediation, not pass: {:?}",
        leg.validate()
    );
    assert!(
        signals[0].contains("install the missing system dependency"),
        "the classifier's remediation has to survive from the baseline record \
         all the way to the user: {}",
        signals[0]
    );
    assert!(
        signals[0].contains("UNRUNNABLE-GATE-RAN"),
        "and the message must name the command that could not run, or the \
         reproduce line points at nothing: {}",
        signals[0]
    );

    let row = leg.validate();
    assert_ne!(
        row.status, "completed",
        "passing here is the defect this leg exists for: {row:?}"
    );

    // The classification is on the *record*, which is what makes it free to
    // re-read on every later attempt instead of paying a second agent call.
    let gate = leg.gate("default");
    assert!(
        !gate.exit_ok,
        "it was still a red measurement — the classification says why, not whether"
    );
    assert!(
        gate.environment.is_some(),
        "the baseline must carry the classification, or nothing downstream can \
         tell this gate from a pre-existing defect: {gate:?}"
    );
}

// ── Leg 2: green before, red now ─────────────────────────────────────────────

/// The other half, and the one that keeps the subtraction from being a way to
/// pass anything. Green at the base, red now: the feature broke it, and the
/// verdict must fire naming the gate's own output.
#[tokio::test]
async fn a_gate_green_at_the_base_and_red_now_does_fail_the_step() {
    let leg = run_leg(
        "regression",
        vec![validate_node(None, 1)],
        GateConfig {
            test_command: Some(regressed("REGRESSION-GATE-RAN")),
            ..Default::default()
        },
    )
    .await;

    let err = leg.validate_error();
    assert!(
        err.contains("REGRESSION-GATE-RAN"),
        "a gate this feature broke must still produce a verdict: {err}"
    );
    assert!(
        leg.gate("default").exit_ok,
        "and the verdict must rest on a baseline that was actually green"
    );
}

// ── Leg 3: red before, differently red now ───────────────────────────────────

/// Red at the base is not a blanket excuse. A gate that says something *more*
/// under this feature's changes is new failures on top of a pre-existing one,
/// and the delta is the feature's to answer for — otherwise a repository with
/// one broken test would be free to break every other one.
#[tokio::test]
async fn a_differently_red_gate_is_still_this_features_verdict() {
    let leg = run_leg(
        "new-atop",
        vec![validate_node(None, 1)],
        GateConfig {
            test_command: Some(red_plus("GATE-RAN", "NEW-FAILURE-LINE")),
            ..Default::default()
        },
    )
    .await;

    let err = leg.validate_error();
    assert!(
        err.contains("NEW-FAILURE-LINE"),
        "new failures atop a pre-existing one are still a verdict: {err}"
    );
    let gate = leg.gate("default");
    assert!(!gate.exit_ok, "the base was red too");
    assert!(
        !gate.fingerprint.is_empty(),
        "a red gate owes the fingerprint the comparison turns on"
    );
}

// ── Leg 3b: and the delta is named, test by test ─────────────────────────────

/// Rung 3. Leg 3 proves a differently-red gate is still a verdict; on its own
/// that verdict says "this whole gate is suspect", and the rework cycle that
/// follows re-derives all of it. With the identifiers each side named, the same
/// verdict says *which* failures are new — the difference between "these 3 of
/// 500 regressed" and "the suite is red".
///
/// The fixture drives both readings out of one command. `@stub-tests` in the
/// gate's own output is what the extractor sees, and `git symbolic-ref` tells the
/// detached baseline worktree from the step's branch checkout — so the base
/// reading is `alpha` and the tip's is `alpha,beta`, with no second commit.
///
/// The assertion is on the *step's* error message because that is the carrier:
/// `VerdictFailure::failing_tests` is rendered by `to_feedback` and threaded into
/// `RetryContext`, which is what a rework template's `{{failing_tests}}` binds.
/// Naming `alpha` there would be the mis-scoping failure — a ticket to fix a test
/// that was already broken before the feature started.
#[tokio::test]
async fn a_differently_red_gate_reports_only_the_failures_that_are_new() {
    let leg = run_leg(
        "scoped",
        vec![validate_node(None, 1)],
        GateConfig {
            test_command: Some(
                "echo 'SCOPED-GATE-RAN'; echo '@stub-tests alpha'; \
                 git symbolic-ref -q HEAD >/dev/null && echo '@stub-tests alpha,beta'; exit 1"
                    .to_string(),
            ),
            ..Default::default()
        },
    )
    .await;

    let gate = leg.gate("default");
    assert_eq!(
        gate.failing_tests.as_deref(),
        Some(["alpha".to_string()].as_slice()),
        "the base's reading is what the delta is taken against, and it is cached \
         on the record so no later attempt re-pays for it"
    );

    let err = leg.validate_error();
    assert!(
        err.contains("Failing tests:") && err.contains("beta"),
        "the new failure has to reach the retry carrier, or the rework cycle \
         re-derives the whole gate: {err}"
    );
    assert!(
        !err.contains("- alpha"),
        "and the pre-existing one must not — a ticket to fix it is work nobody \
         asked for, on a defect this feature did not cause: {err}"
    );
    assert!(
        err.contains("SCOPED-GATE-RAN"),
        "the scope is added to the evidence, never substituted for it: {err}"
    );
}

// ── Leg 4: the exclusion is named beside an attributable failure ─────────────

/// A subtraction the user cannot audit is one they will not trust the first
/// time it is wrong. With one gate excluded and another genuinely broken, the
/// verdict must name both — the failure to fix, *and* the failure it is
/// deliberately not asking about. Without the second half, an implementer who
/// can see a red `stale` gate in the log has every reason to go and fix it:
/// work nobody asked for, on a defect this feature did not cause.
#[tokio::test]
async fn an_excluded_gate_is_named_beside_the_failure_that_is_being_charged() {
    let leg = run_leg(
        "mixed",
        vec![validate_node(None, 1)],
        GateConfig {
            harnesses: vec![
                ("stale".to_string(), always_red("STALE-GATE-RAN")),
                ("broken".to_string(), regressed("BROKEN-GATE-RAN")),
            ],
            validation_gates: Some(vec!["stale".to_string(), "broken".to_string()]),
            ..Default::default()
        },
    )
    .await;

    let err = leg.validate_error();
    assert!(
        err.contains("BROKEN-GATE-RAN"),
        "the attributable gate is the verdict: {err}"
    );
    assert!(
        err.contains("'stale'") && err.contains("NOT part of this verdict"),
        "the exclusion must be named where the failure would have been: {err}"
    );
    assert!(
        !leg.gate("stale").exit_ok && leg.gate("broken").exit_ok,
        "both gates were measured at the base, which is what told them apart"
    );
}

// ── Leg 5: C6 is not consulted about what the measurement answered ───────────

/// The narrowing. This is `harness_triage.rs`'s environment fixture with one
/// thing changed: the baseline says the gate was **already red, differently**
/// at the base. The failure still reproduces unchanged across attempts, and its
/// output still carries `@stub-triage environment` — so with no baseline the
/// classifier would fire and terminate the run immediately.
///
/// It must not, because the measurement already answered: the gate reached an
/// exit status at the base, so the machine can run it, and the output changed
/// under this feature's changes. The run therefore burns its retry budget as an
/// ordinary verdict would, and no `EnvironmentNotReady` signal is emitted.
///
/// The baseline's *own* triage call still happens — every red baseline gate is
/// classified — but it is asked about the base's output, which is the `GATE-RAN`
/// line without the directive (`red_plus` emits the extra only on a branch, and
/// HB2b measures detached). So it answers `regression`, the gate stays
/// subtractable, and nothing escalates. That asymmetry is exactly the one rung 3
/// turns on: the classification describes the failure that was *measured*, never
/// the one observed later.
#[tokio::test]
async fn the_triage_agent_is_not_asked_about_a_case_the_baseline_settled() {
    let leg = run_leg(
        "narrowed",
        vec![validate_node(Some("s-validate"), 2)],
        GateConfig {
            test_command: Some(red_plus("GATE-RAN", "@stub-triage environment")),
            ..Default::default()
        },
    )
    .await;

    assert!(
        leg.signals.environment_not_ready.lock().unwrap().is_empty(),
        "the classifier must not have been consulted, so nothing can have \
         escalated: {:?}",
        leg.signals.environment_not_ready.lock().unwrap()
    );
    assert_eq!(
        leg.signals.retry_budget_exhausted.lock().unwrap().len(),
        1,
        "the failure stays on the ordinary verdict path and exhausts the budget"
    );
    assert!(
        !leg.gate("default").exit_ok,
        "the baseline that settled it was a red one"
    );
}

// ── The record's other consumer: what the spec is told it can prove ──────────

/// A spec-shaped step whose prompt asks for the briefing and nothing else, so
/// the block's presence in the rendered prompt is unambiguous.
fn spec_node(marker: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "s-spec",
        "kind": "agent",
        "title": "Draft Spec",
        "agent_kind": "stub",
        "prompt_template": format!("{marker}\n{{{{harness_baseline}}}}\n"),
        "capability": "artifacts",
        "max_iterations": 1
    })
}

/// `{{harness_baseline}}` must reach the spec step carrying the **commands that
/// will actually judge the work**. Both failed validate attempts in
/// `f-1785157902856` cost a rework cycle because the criteria named commands
/// the harness never ran; `{{test_command}}`, which the prompt used to
/// interpolate, stopped being that answer when harnesses went plural.
///
/// An unbound token renders as the empty string, so an assertion that the
/// command is *there* is exactly the assertion that the binding happened.
#[tokio::test]
async fn the_spec_step_is_told_which_commands_can_prove_a_criterion() {
    const MARKER: &str = "SPEC-BRIEFING-LEG-CONFIGURED";
    let leg = run_leg(
        "briefing",
        vec![spec_node(MARKER), validate_node(None, 1)],
        GateConfig {
            harnesses: vec![(
                "unit".to_string(),
                "echo 'BRIEFING-GATE-CMD'; echo '@stub-verdict verdict'".to_string(),
            )],
            validation_gates: Some(vec!["unit".to_string()]),
            ..Default::default()
        },
    )
    .await;

    let prompts = prompts_containing(MARKER);
    assert_eq!(prompts.len(), 1, "one spec turn");
    let prompt = &prompts[0];
    assert!(
        prompt.contains("BRIEFING-GATE-CMD"),
        "the gate's command must reach the spec author verbatim:\n{prompt}"
    );
    assert!(
        prompt.contains("only"),
        "and it must say these are the only commands executed, or a criterion \
         against some other command still looks provable:\n{prompt}"
    );
    // The baseline node is absent here, so nothing measured the gates — which
    // must read as *unknown*, never as a pass.
    assert!(
        prompt.contains("not measured"),
        "absent is not green, in the prompt as well as in the verdict:\n{prompt}"
    );
    assert_eq!(
        leg.steps
            .iter()
            .find(|(id, _)| id == "s-spec")
            .map(|(_, row)| row.status.clone()),
        Some("completed".to_string())
    );
}

/// The `not_configured` row, reached at spec time. Today an unprovable
/// criterion can only be discovered at validate — after the entire implement
/// budget is gone, and only if the agent then picks `environment` over `fail`.
/// A project with no gate configured is knowable here, for the price of a
/// paragraph.
#[tokio::test]
async fn a_project_with_no_gate_is_told_so_before_a_criterion_is_written() {
    const MARKER: &str = "SPEC-BRIEFING-LEG-UNCONFIGURED";
    run_leg(
        "briefing-none",
        vec![spec_node(MARKER), validate_node(None, 1)],
        GateConfig::default(),
    )
    .await;

    let prompts = prompts_containing(MARKER);
    assert_eq!(prompts.len(), 1, "one spec turn");
    let prompt = &prompts[0];
    assert!(
        prompt.contains("NOTHING"),
        "an unconfigured harness must be unmissable:\n{prompt}"
    );
    assert!(
        prompt.contains("never be shown MET"),
        "and the consequence for a criterion is the whole reason to say it \
         early:\n{prompt}"
    );
}

// ── The row with no baseline column at all ───────────────────────────────────

/// exit 127 is terminal **whether or not** the binary was equally missing at
/// the base. Decision 44's table gives it its own row for a reason: the code
/// never ran, so there is nothing for a subtraction to be evidence about, and
/// "pre-existing" would quietly pass a step that tested nothing. Which is
/// exactly what happens if the fast path is moved behind the subtraction — a
/// missing binary is red at the base with the identical diagnostic, so it is
/// the *most* subtractable failure there is.
///
/// `eval` is the fixture's lever: HB1's launch preflight probes the head binary
/// of each stage and skips shell builtins, so this reaches validate rather than
/// being blocked at launch — while `detect_missing_command` still sees the
/// missing name as a token of the command it was asked about.
#[tokio::test]
async fn a_missing_binary_stays_terminal_rather_than_being_subtracted() {
    let leg = run_leg(
        "missing-binary",
        vec![validate_node(None, 1)],
        GateConfig {
            test_command: Some("eval demeteo-absent-binary-hb2c".to_string()),
            ..Default::default()
        },
    )
    .await;

    let signals = leg.signals.environment_not_ready.lock().unwrap().clone();
    assert_eq!(
        signals.len(),
        1,
        "a missing binary is an environment failure with remediation, not a \
         verdict and not a subtraction: {:?}",
        leg.validate()
    );
    assert!(
        signals[0].contains("demeteo-absent-binary-hb2c"),
        "the remediation must name the binary: {}",
        signals[0]
    );
    assert!(
        leg.baseline.is_none(),
        "and nothing should have been measured: the fast path fires before the \
         baseline is consulted at all, got {:?}",
        leg.baseline
    );
}
