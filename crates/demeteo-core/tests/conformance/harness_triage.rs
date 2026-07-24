//! Harness-failure triage driver-integration fixtures (C6,
//! `docs/EXECUTION_CONSISTENCY_PLAN.md`).
//!
//! The C6 pure helpers (fingerprint normalization both directions,
//! `should_triage`, `parse_triage_text`, `build_environment_message`) are unit-
//! tested next to their definitions in `driver/verifier.rs`. What those unit
//! tests *cannot* cover is the wiring: that a red harness inside a real running
//! feature actually
//!
//! 1. does **not** triage on first sight (attempt 1 → `Verdict` → retry), and
//! 2. **does** triage once the failure reproduces unchanged, and
//! 3. routes an `environment` verdict to a **terminal** failure that bypasses
//!    the `evaluate_on_failure` retry budget (the DoD's "Environment never
//!    reaches evaluate_on_failure"), while
//! 4. a `regression` verdict stays on the `Verdict` path and burns the budget
//!    as before (the classifier fail-safe direction).
//!
//! Driving all of that end-to-end needs a real agent runtime for the triage
//! classifier. This is exactly the seam C5's [`StubRuntime`] provides: the
//! triage agent runs on the feature's own `agent_kind` (here `"stub"`), and a
//! failing harness command whose output carries a `@stub-triage <category>`
//! line makes the stub return that category deterministically — no LLM, no
//! Docker. See the `stub_runtime` module docs for the directive protocol.
//!
//! Both fixtures use the **same** single-step workflow and the **same**
//! persistent (reproduces-identically) failing harness command; they differ
//! only in the `@stub-triage` category the command emits, and assert on which
//! terminal path the run took via a recording [`NotificationPort`].

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

const REPO_PATH: &str = "demeteo/triage";
const PROVIDER_ID: &str = "triage-provider";

/// Records whether the two terminal-signal events the C6 paths emit were seen,
/// so a test can assert which branch the run actually took. Everything else is
/// ignored.
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

/// A single agent step that carries a `verifier` config and loops back to
/// itself on failure. The verifier's harness command is the project's
/// `test_command` (set by [`set_failing_harness`]); when it exits non-zero the
/// harness-first gate short-circuits the step *before* any agent turn, so the
/// step body itself never needs to do anything. `max_iterations: 2` gives the
/// failure exactly one retry — enough to reproduce and trip the persistence
/// gate.
fn triage_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Harness Triage Conformance",
        "description": "One agent step whose harness always fails, to exercise C6 triage.",
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
                    "harness_name": null,
                    "verdict_key": "verdict"
                },
                "on_failure": "s-validate",
                "max_iterations": 2
            }
        ]
    })
}

/// Point the project's `test_command` at a deterministic failing command whose
/// output carries a standalone `@stub-triage <category>` line. The output is
/// byte-stable across attempts (no timestamps / paths / volatile ids), so the
/// second attempt's fingerprint matches the first and the persistence gate
/// fires. The triage prompt embeds this output, so the stub classifier reads
/// the category back out. `set -e`-free, plain `sh` builtins only.
fn set_failing_harness(
    ctx: &AppContext,
    project_id: &crate::domain::ids::ProjectId,
    category: &str,
) {
    // A fresh project has no persisted settings row (defaults are applied
    // lazily), so build one from the engine default and point its harness at a
    // deterministic failing command.
    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project_id.clone();
    settings.worktree_strategy.test_command = Some(format!(
        "echo 'error: the system library gdk-3.0 was not found'; \
         echo '@stub-triage {category}'; exit 1"
    ));
    ctx.projects.save_settings(settings).expect("save settings");
}

/// Seed a real local git repo at the project's expected repo dir so
/// `bootstrap_project` skips its (network) clone — the same "already cloned"
/// shortcut every offline path relies on (mirrors the topology gate).
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
    std::fs::write(dir.join("README.md"), "# triage conformance fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

/// Drive a freshly-started feature to a terminal state and return its final
/// status plus the failing step's final error message. Fails the test if it
/// doesn't settle within the timeout.
async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> (String, String) {
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
            let err = ctx
                .features
                .steps_for_feature(feature_id)
                .unwrap_or_default()
                .into_iter()
                .find(|s| s.step_id.0 == "s-validate")
                .and_then(|s| s.error_message)
                .unwrap_or_default();
            return (feature.status, err);
        }
        if started.elapsed() > MAX_WAIT {
            let steps = ctx
                .features
                .steps_for_feature(feature_id)
                .unwrap_or_default();
            panic!(
                "feature {} did not reach a terminal state in {:?}; last status {}, steps: {:#?}",
                feature_id.as_str(),
                MAX_WAIT,
                feature.status,
                steps
                    .iter()
                    .map(|s| (
                        s.step_id.0.clone(),
                        s.status.clone(),
                        s.error_message.clone()
                    ))
                    .collect::<Vec<_>>()
            );
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

/// Register provider, create project (with the failing harness), seed the repo,
/// bootstrap, ingest [`triage_workflow`], and drive one feature to a terminal
/// state on a locally-executing engine. Returns the terminal feature status,
/// the failing step's error message, and the recorded terminal signals.
/// Returns the terminal (status, error), the signal recorder, and the
/// validate step's `step_attempts` rows (V31, P1.8) — one per driver
/// dispatch, so the triage legs can assert per-attempt history without
/// re-running the whole leg.
async fn run_triage_leg(
    triage_category: &str,
) -> (
    String,
    String,
    Arc<SignalRecorder>,
    Vec<crate::domain::models::StepAttempt>,
) {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let recorder = Arc::new(SignalRecorder::default());
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-triage-{triage_category}-{}",
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
            name: "triage".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    set_failing_harness(&ctx, &project.id, triage_category);
    init_local_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &triage_workflow()).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Triage Feature",
            "Exercise the C6 harness-failure triage path.",
            Some("stub"),
            // model / effort / commit_artifacts: inherit.
            None,
            None,
            None,
            Some(2),
            // max_budget_usd: inherit.
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start");

    let (status, err) = poll_terminal(&ctx, &feature.id).await;
    // Per-attempt history for the (single) validate step, read before the
    // context is dropped.
    let attempts = ctx
        .features
        .steps_for_feature(&feature.id)
        .expect("list steps")
        .into_iter()
        .find(|s| s.step_id.0 == "s-validate")
        .map(|s| {
            ctx.features
                .attempts_for_step(&s.id)
                .expect("list step attempts")
        })
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&tmp);
    (status, err, recorder, attempts)
}

/// The headline C6 fixture: a persistent (reproduces-unchanged) harness failure
/// triaged as `environment` terminates the run *immediately* — it never reaches
/// the retry budget. Because the environment verdict only terminates on
/// attempt 2 (the first attempt retries with no triage), reaching a terminal
/// `failed` state here also proves triage did **not** fire on attempt 1: had it
/// fired then, the run would have terminated after zero retries, and had it
/// never fired, the run would have exhausted the budget and emitted
/// `RetryBudgetExhausted` instead.
#[tokio::test]
async fn persistent_environment_failure_terminates_without_exhausting_budget() {
    let (status, err, rec, attempts) = run_triage_leg("environment").await;

    assert_eq!(status, "failed", "environment triage must fail the feature");
    assert_eq!(
        rec.environment_not_ready.lock().unwrap().len(),
        1,
        "exactly one EnvironmentNotReady signal must fire; err was: {err}"
    );
    assert!(
        rec.retry_budget_exhausted.lock().unwrap().is_empty(),
        "environment triage must NOT reach evaluate_on_failure's budget path",
    );
    // The terminal message is the user-facing remediation built by
    // `build_environment_message`: it names the reproduce line and the
    // remediation, not a bare build-log tail.
    assert!(
        err.contains("Environment not ready") && err.contains("Reproduce:"),
        "step error must be the environment remediation message; got: {err}"
    );
    // Per-attempt history (P1.8): the same run leaves one closed row per
    // dispatch, with *distinct* failure classes — a plain verdict failure
    // on attempt 1, the triaged environment termination on attempt 2 —
    // and each row names the retry-policy rule that answered it (P1.10).
    let summary: Vec<(u32, &str, Option<&str>, Option<&str>)> = attempts
        .iter()
        .map(|a| {
            (
                a.attempt_no,
                a.status.as_str(),
                a.error_class.as_deref(),
                a.applied_rule.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (1, "failed", Some("verdict"), Some("verdict.redirect")),
            (
                2,
                "failed",
                Some("non_retryable"),
                Some("non_retryable.fail")
            ),
        ],
        "environment leg must record two attempts with distinct error classes"
    );
    assert!(
        attempts.iter().all(|a| a.ended_at.is_some()),
        "every attempt row must be closed"
    );
}

/// The fail-safe direction: a persistent failure the classifier calls a
/// `regression` stays on the `Verdict` retry path and exhausts the budget as
/// before — no `Environment` terminate, no premature stop. This is what
/// guarantees a broken triage (or a genuine regression) can only ever *withhold*
/// an escalation, never manufacture one.
#[tokio::test]
async fn persistent_regression_failure_exhausts_retry_budget() {
    let (status, err, rec, attempts) = run_triage_leg("regression").await;

    assert_eq!(status, "failed", "regression must still fail the feature");
    let budget_events = rec.retry_budget_exhausted.lock().unwrap();
    assert_eq!(
        budget_events.len(),
        1,
        "regression must burn the retry budget and emit exactly one RetryBudgetExhausted; err was: {err}"
    );
    assert!(
        budget_events[0].contains("retry budget exhausted"),
        "the budget-exhausted signal must name the exhaustion; got: {}",
        budget_events[0]
    );
    assert!(
        rec.environment_not_ready.lock().unwrap().is_empty(),
        "a regression verdict must never fire EnvironmentNotReady",
    );
    // The failing step's own final error is the harness verdict tail (the
    // budget-exhausted framing rides the notification, not the step row).
    assert!(
        err.contains("exited with failure"),
        "step error must carry the harness failure; got: {err}"
    );
    // Per-attempt history (P1.8): every dispatch of the redirect loop is
    // its own closed row — the two budgeted retries plus the try that
    // exhausted the budget, all classed as verdict failures and all
    // answered by the same redirect rule (P1.10; exhaustion is that
    // rule's spent budget, not a different rule).
    let summary: Vec<(u32, &str, Option<&str>, Option<&str>)> = attempts
        .iter()
        .map(|a| {
            (
                a.attempt_no,
                a.status.as_str(),
                a.error_class.as_deref(),
                a.applied_rule.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        summary,
        vec![
            (1, "failed", Some("verdict"), Some("verdict.redirect")),
            (2, "failed", Some("verdict"), Some("verdict.redirect")),
            (3, "failed", Some("verdict"), Some("verdict.redirect")),
        ],
        "regression leg must record one attempt row per dispatch"
    );
    assert!(
        attempts.iter().all(|a| a
            .failure_fingerprint
            .as_deref()
            .is_some_and(|f| !f.is_empty())),
        "failed attempts must carry a failure fingerprint"
    );
}
