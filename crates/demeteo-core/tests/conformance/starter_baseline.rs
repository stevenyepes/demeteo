//! Baseline behavioral harness for the nine bundled starter workflows
//! (P0.2, `docs/TASKS_DAG_WORKFLOWS.md`).
//!
//! Every starter is *executed* through the engine with the deterministic
//! [`StubRuntime`](crate::adapters::agent::stub_runtime) and reduced to a
//! golden [`StarterSnapshot`] committed beside this file. This is the
//! regression gate every Phase-1 engine task (schema v2, registry,
//! ready-set scheduler, retry policy) runs against: chains are DAGs, so a
//! representation change must not change any of these snapshots.
//!
//! ## What is baselined — and what is not
//!
//! The snapshot captures *engine behavior*: the ordered step set, each
//! step's kind and terminal status, iteration counts, normalized error
//! messages, the feature's terminal status, and every materialized declared
//! artifact's body. It deliberately excludes wall-clock, ids, and absolute
//! paths (normalized out via [`normalize`]).
//!
//! Prompt *text* is not baselined: the harness mechanically injects stub
//! directives into each starter before ingest ([`augment_starter`]) —
//! `@stub-write <path>` per declared `last_write_to` artifact and
//! `@stub-verdict <key>` into verifier instructions — because the stub
//! agent is directive-driven and the starters' real prompts address LLMs.
//! Structure (steps, kinds, `on_failure`, `task_list_from`, capabilities,
//! artifact declarations) is ingested verbatim.
//!
//! Gates are auto-approved the moment they suspend, mirroring a user who
//! always clicks Approve.
//!
//! ## Regenerating snapshots
//!
//! ```sh
//! UPDATE_SNAPSHOTS=1 cargo test -p demeteo-core starter_baseline
//! git diff crates/demeteo-core/tests/conformance/snapshots/  # review!
//! ```
//!
//! A diff in a snapshot is a *behavior change* — it must be either a bug in
//! your change or an explicitly intended semantic change reviewed as such.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::step_executor::FeatureLaunch;
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/starter-baseline";
const PROVIDER_ID: &str = "starter-baseline-provider";

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// The transport- and run-independent reduction of one starter's execution
/// this suite snapshots. Field order is the serialized order — keep it
/// stable so snapshot diffs stay readable.
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StarterSnapshot {
    starter: String,
    terminal_status: String,
    /// Workflow order (by `step_index`).
    steps: Vec<StepSnapshot>,
    /// `(step_id, artifact-basename, body)`, sorted for order-independent
    /// comparison.
    artifacts: Vec<(String, String, String)>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
struct StepSnapshot {
    step_id: String,
    kind: String,
    status: String,
    iterations: u32,
    /// `error_message` with volatile substrings (ids, absolute paths)
    /// replaced by placeholders; `null` for a clean step.
    error: Option<String>,
}

/// Directory holding the committed golden snapshots.
fn snapshots_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("conformance")
        .join("snapshots")
        .join("starter_baseline")
}

/// The bundled starter JSON, loaded from the repo's `src-tauri/workflows/`
/// (the same files `seed_starter_workflows` ships in the binary).
fn load_starter(name: &str) -> serde_json::Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../src-tauri/workflows")
        .join(format!("{name}.json"));
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read starter {}: {e}", path.display()));
    serde_json::from_str(&body).unwrap_or_else(|e| panic!("parse starter {name}: {e}"))
}

/// Mechanically inject stub directives so the directive-driven stub agent
/// can satisfy each step's declared contract. Structure is untouched.
fn augment_starter(mut wf: serde_json::Value) -> serde_json::Value {
    let steps = wf["steps"].as_array_mut().expect("starter has steps");
    for step in steps {
        // One `@stub-write` per declared `last_write_to` artifact, so the
        // declared-artifact resolver finds a materialized deliverable.
        let mut writes = String::new();
        if let Some(decls) = step["artifacts"].as_array() {
            for decl in decls {
                if decl["capture"]["kind"] == "last_write_to" {
                    if let Some(path) = decl["capture"]["path"].as_str() {
                        writes.push_str(&format!("\n@stub-write {path}"));
                    }
                }
            }
        }
        // …and one to a *source* path for every step that implements.
        //
        // An implement step's deliverable is the commit, not a file it was
        // told to name, so nothing above emits a directive for one — and a
        // stub that writes nothing leaves the branch empty, which the
        // sequence step's no-op guard correctly fails on the first attempt.
        // The starter would then baseline "implement fails, then recovers"
        // as its expected shape, and stop proving the first-attempt path for
        // every implement step in every starter.
        //
        // Deliberately outside the artifact subdir: a write under
        // `artifacts/` is what `judge_stage` calls a stranded deliverable,
        // so a path there would exercise the failure it is meant to avoid.
        if step["capability"] == "implement" {
            let id = step["id"].as_str().unwrap_or("step");
            writes.push_str(&format!("\n@stub-write src/{id}.txt"));
        }
        if !writes.is_empty() {
            if let Some(prompt) = step["prompt_template"].as_str() {
                step["prompt_template"] = format!("{prompt}\n{writes}\n").into();
            }
        }
        // `@stub-verdict` into verifier instructions: the verifier prompt
        // embeds them, so the stub verifier turn returns a passing verdict.
        if step["verifier"].is_object() {
            let key = step["verifier"]["verdict_key"]
                .as_str()
                .unwrap_or("verdict")
                .to_string();
            if let Some(instr) = step["verifier"]["instructions"].as_str() {
                step["verifier"]["instructions"] =
                    format!("{instr}\n\n@stub-verdict {key}\n").into();
            }
        }
    }
    wf
}

/// Run `git` in `dir`, failing the test on a non-zero exit.
fn git_in(dir: &Path, args: &[&str]) {
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

/// Seed a real local git repo at the project's expected repo dir so
/// `bootstrap_project` skips its (network) clone — the same "already
/// cloned" shortcut the topology and triage gates use.
fn init_local_repo(dir: &Path) {
    std::fs::create_dir_all(dir).expect("create repo dir");
    let git = |args: &[&str]| git_in(dir, args);
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# starter baseline fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

/// Drive the feature to a terminal state, auto-approving every gate the
/// moment it suspends (a user who always clicks Approve). Returns the
/// terminal feature status.
async fn poll_terminal_approving_gates(ctx: &AppContext, feature_id: &FeatureId) -> String {
    const MAX_WAIT: Duration = Duration::from_secs(180);
    let started = Instant::now();
    let mut decided: std::collections::HashSet<String> = std::collections::HashSet::new();
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
        for s in ctx
            .features
            .steps_for_feature(feature_id)
            .unwrap_or_default()
        {
            if s.status == "awaiting_gate" && !decided.contains(&s.id.0) {
                match ctx.presenter.gate_decide(&s.id.0, "approve", None).await {
                    Ok(()) => {
                        decided.insert(s.id.0.clone());
                    }
                    // A decide can race the gate's own bookkeeping; retry
                    // on the next poll tick rather than failing the run.
                    Err(e) => eprintln!("[starter-baseline] gate_decide retry: {e:?}"),
                }
            }
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

/// Replace run-specific substrings (temp dirs, ids, slugs) with stable
/// placeholders so error messages and artifact bodies compare across runs.
fn normalize(text: &str, volatile: &[(String, &str)]) -> String {
    let mut out = text.to_string();
    for (needle, placeholder) in volatile {
        if !needle.is_empty() {
            out = out.replace(needle.as_str(), placeholder);
        }
    }
    out
}

/// Execute one starter end-to-end under the stub agent and reduce it to its
/// [`StarterSnapshot`].
async fn run_starter(name: &str) -> StarterSnapshot {
    run_starter_with(name, |_| {}).await
}

/// [`run_starter`], with `configure` applied to the project settings after the
/// harness's own defaults and before they are saved.
async fn run_starter_with(
    name: &str,
    configure: impl FnOnce(&mut crate::domain::models::ProjectSettings),
) -> StarterSnapshot {
    run_starter_launched(name, configure, init_local_repo, |_| {})
        .await
        .snapshot
}

/// A launched starter's snapshot, plus the stored baseline record the snapshot
/// leaves out: which commit it names is run-specific, so it cannot be golden.
struct LaunchedStarter {
    snapshot: StarterSnapshot,
    harness_baseline: Option<crate::domain::harness_baseline::HarnessBaseline>,
    /// Checkouts the baseline node provisioned and did not tear down.
    node_worktrees: Vec<String>,
    /// Every environment-not-ready notification the run raised.
    environment_not_ready: Vec<String>,
}

/// [`run_starter_with`], with the fixture repo built by `seed_repo` at the
/// project's repo dir instead of [`init_local_repo`], and `launch` applied to
/// the [`FeatureLaunch`] before it starts.
async fn run_starter_launched(
    name: &str,
    configure: impl FnOnce(&mut crate::domain::models::ProjectSettings),
    seed_repo: impl FnOnce(&Path),
    launch: impl FnOnce(&mut FeatureLaunch),
) -> LaunchedStarter {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-starter-{name}-{}",
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
            name: format!("starter-{name}"),
            compute_type: "local".to_string(),
            remote_host: None,
            repos: vec![projects::RepositoryConfig {
                repo_path: REPO_PATH.to_string(),
                provider_id: PROVIDER_ID.to_string(),
            }],
        },
    )
    .expect("create project");

    // A fresh project has no persisted settings row (defaults are applied
    // lazily), but the finalize step requires one — seed the engine default,
    // as the triage suite does. The default `test_command` (`npm test`)
    // cannot run in the README-only fixture repo, so point the harness at a
    // deterministically green command: verifier steps then exercise the
    // real harness-first gate (green harness → verifier agent turn) instead
    // of dying on an ENOENT.
    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = project.id.clone();
    settings.worktree_strategy.test_command = Some("true".to_string());
    configure(&mut settings);
    ctx.projects.save_settings(settings).expect("save settings");

    seed_repo(&paths::repo_target_dir_local(
        &ctx.workspace_dir,
        project.id.as_str(),
        REPO_PATH,
    ));
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow = augment_starter(load_starter(name));
    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &workflow).expect("ingest starter");

    let mut feature_launch = FeatureLaunch {
        project_id: project.id.0.clone(),
        workflow_id: workflow_id.0.clone(),
        title: format!("Baseline {name}").clone(),
        description: "Deterministic starter-baseline run under the stub agent.".to_string(),
        agent_kind: Some("stub".to_string()),
        ..Default::default()
    };
    launch(&mut feature_launch);
    let feature = ctx
        .executor
        .feature_start(feature_launch)
        .await
        .expect("feature_start");

    let terminal_status = poll_terminal_approving_gates(&ctx, &feature.id).await;
    let harness_baseline = ctx
        .features
        .get(&feature.id)
        .expect("feature read")
        .and_then(|f| f.harness_baseline);
    // Linked worktrees are siblings of the clone (`<repo>_wt_<id>`).
    let repo_dir = paths::repo_target_dir_local(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    let node_worktrees = repo_dir
        .parent()
        .and_then(|parent| std::fs::read_dir(parent).ok())
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with("-baseline-node"))
        .collect();
    let environment_not_ready = ctx
        .notifications
        .list(None, 100)
        .expect("notifications read")
        .into_iter()
        .filter(|n| {
            n.kind == crate::domain::models::NotificationKind::EnvironmentNotReady
                && n.feature_id == feature.id.0
        })
        .map(|n| n.message)
        .collect();

    // Everything run-specific that can leak into error messages or
    // artifact bodies, longest first so path prefixes don't shadow ids.
    let volatile: Vec<(String, &str)> = vec![
        (
            ctx.workspace_dir.to_string_lossy().into_owned(),
            "<workspace>",
        ),
        (tmp.to_string_lossy().into_owned(), "<app-data>"),
        (feature.id.0.clone(), "<feature-id>"),
        (project.id.0.clone(), "<project-id>"),
    ];

    let mut steps = ctx
        .features
        .steps_for_feature(&feature.id)
        .expect("steps read");
    steps.sort_by_key(|s| s.step_index);

    let mut step_snaps = Vec::new();
    let mut artifacts: Vec<(String, String, String)> = Vec::new();
    for s in &steps {
        step_snaps.push(StepSnapshot {
            step_id: s.step_id.0.clone(),
            kind: s.step_kind.clone(),
            status: s.status.clone(),
            iterations: s.iteration_count,
            error: s.error_message.as_deref().map(|e| normalize(e, &volatile)),
        });
        for path in &s.artifact_paths {
            let body = ctx
                .run_view
                .artifact_body("local", path)
                .await
                .unwrap_or_else(|e| format!("<unreadable: {e}>"));
            let base = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path)
                .to_string();
            artifacts.push((s.step_id.0.clone(), base, normalize(&body, &volatile)));
        }
    }
    artifacts.sort();

    let _ = std::fs::remove_dir_all(&tmp);
    LaunchedStarter {
        snapshot: StarterSnapshot {
            starter: name.to_string(),
            terminal_status,
            steps: step_snaps,
            artifacts,
        },
        harness_baseline,
        node_worktrees,
        environment_not_ready,
    }
}

/// Compare against (or, with `UPDATE_SNAPSHOTS=1`, rewrite) the committed
/// golden snapshot.
async fn assert_starter_baseline(name: &str) {
    let actual = run_starter(name).await;
    let path = snapshots_dir().join(format!("{name}.json"));

    if matches!(std::env::var("UPDATE_SNAPSHOTS"), Ok(v) if !v.is_empty() && v != "0") {
        std::fs::create_dir_all(snapshots_dir()).expect("create snapshots dir");
        let body = serde_json::to_string_pretty(&actual).expect("serialize snapshot");
        std::fs::write(&path, body + "\n").expect("write snapshot");
        eprintln!("[starter-baseline] snapshot updated: {}", path.display());
        return;
    }

    let committed = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "no committed snapshot at {} ({e}); run UPDATE_SNAPSHOTS=1 cargo test -p \
             demeteo-core starter_baseline to create it",
            path.display()
        )
    });
    let expected: StarterSnapshot =
        serde_json::from_str(&committed).expect("parse committed snapshot");
    assert_eq!(
        expected, actual,
        "starter '{name}' diverged from its committed baseline snapshot — this is a \
         behavior change; if intended, regenerate with UPDATE_SNAPSHOTS=1 and review the diff"
    );
}

#[tokio::test]
async fn starter_baseline_simple_task() {
    assert_starter_baseline("simple-task").await;
}

#[tokio::test]
async fn starter_baseline_bugfix_pipeline() {
    assert_starter_baseline("bugfix-pipeline").await;
}

#[tokio::test]
async fn starter_baseline_ci_fix() {
    assert_starter_baseline("ci-fix").await;
}

#[tokio::test]
async fn starter_baseline_docs_update() {
    assert_starter_baseline("docs-update").await;
}

#[tokio::test]
async fn starter_baseline_experiment() {
    assert_starter_baseline("experiment").await;
}

#[tokio::test]
async fn starter_baseline_refactor() {
    assert_starter_baseline("refactor").await;
}

#[tokio::test]
async fn starter_baseline_code_review() {
    assert_starter_baseline("code-review").await;
}

#[tokio::test]
async fn starter_baseline_standard_feature_pipeline() {
    assert_starter_baseline("standard-feature-pipeline").await;
}

#[tokio::test]
async fn starter_baseline_address_review() {
    assert_starter_baseline("address-review").await;
}

/// A red branch ends `code-review`'s `s-validate-branch` before its agent
/// turn: `run_harness_first` fails the step, so `branch-validation.md` is never
/// written and the gate's output survives only in the step's `error_message`.
/// `s-review`'s report is already written by then — that is what the fix
/// launch reads the red gate out of.
///
/// The *prepare* is reddened, not the test command. A constant-red
/// `test_command` fails identically at the merge-base, so
/// `adjudicate_red_gates` subtracts it as pre-existing and the turn runs:
/// observed with `test_command = "echo …; exit 1"` and no prepare, both steps
/// complete, both reports are written and the run ends `awaiting_mr`. A
/// failing prepare is never subtracted, and triage is not consulted on a first
/// attempt.
#[tokio::test]
async fn a_red_branch_ends_the_review_gate_step_without_its_report() {
    const MARKER: &str = "starter-baseline-red-prepare";
    let run = run_starter_with("code-review", |settings| {
        settings.worktree_strategy.prepare_command = Some(format!("echo {MARKER}; exit 1"));
    })
    .await;

    let step = |id: &str| {
        run.steps
            .iter()
            .find(|s| s.step_id == id)
            .unwrap_or_else(|| panic!("no step {id} in {run:#?}"))
    };
    let has_artifact = |id: &str, file: &str| {
        run.artifacts
            .iter()
            .any(|(step_id, base, _)| step_id == id && base == file)
    };

    assert_eq!(step("s-review").status, "completed", "{run:#?}");
    assert!(has_artifact("s-review", "code-review.md"), "{run:#?}");

    let gate = step("s-validate-branch");
    assert_eq!(gate.status, "failed", "{run:#?}");
    assert!(
        !has_artifact("s-validate-branch", "branch-validation.md"),
        "{run:#?}"
    );
    assert!(
        gate.error.as_deref().is_some_and(|e| e.contains(MARKER)),
        "the gate step's error_message must carry the prepare's output: {run:#?}"
    );

    assert_eq!(run.terminal_status, "failed", "{run:#?}");
}

/// A project with no `prepare_command` and no `test_command` gives
/// `s-validate-branch` nothing to run, and that branch is unjudged, not red.
/// The run has to complete and keep both reports. An `environment` verdict
/// here maps to `VerdictDisposition::Unjudgeable`, which ends the step without
/// recording its artifact and fails the feature, so every review of an
/// unconfigured project would end `failed` and lose `branch-validation.md`.
///
/// This proves the engine half only: the stub agent always answers `pass`, so
/// what is asserted is that a pass with nothing configured completes and
/// records the report. That the starter asks its verifier for that pass is
/// held by `the_review_starter_passes_a_branch_no_gate_was_configured_for`
/// in `src-tauri/tests/infrastructure/workflows.rs`.
#[tokio::test]
async fn a_review_with_no_gate_configured_completes_with_both_reports() {
    let run = run_starter_with("code-review", |settings| {
        settings.worktree_strategy.test_command = None;
        settings.worktree_strategy.prepare_command = None;
    })
    .await;

    let step = |id: &str| {
        run.steps
            .iter()
            .find(|s| s.step_id == id)
            .unwrap_or_else(|| panic!("no step {id} in {run:#?}"))
    };

    assert_eq!(step("s-review").status, "completed", "{run:#?}");
    assert_eq!(step("s-validate-branch").status, "completed", "{run:#?}");
    assert!(
        run.artifacts
            .iter()
            .any(|(step_id, base, _)| step_id == "s-validate-branch"
                && base == "branch-validation.md"),
        "{run:#?}"
    );
    assert_ne!(run.terminal_status, "failed", "{run:#?}");
}

/// A same-repo fix run is measured against what the reviewed pull request
/// targets, so a gate the pull request broke stays the run's to close.
///
/// `main` carries `HEALTHY` and `pr-head` removes it, so the gate is green at
/// the target and red on the head the fix branch is cut from. The stub
/// addresses nothing, so `s-confirm`'s harness-first pass is red, and
/// `adjudicate_red_gates` measures that at `merge_base(main, fix branch)`,
/// where the gate is green: the failure is the run's, and the run fails before
/// `s-finalize` can publish.
///
/// Watched red with `diff_base_branch: Some("pr-head")`, which is what
/// `planFixLaunch` used to send. The merge-base is then the head itself, where
/// the gate is equally red, so it is subtracted as pre-existing. The verifier
/// passes, `s-finalize` completes, and the run ends `awaiting_mr` with the
/// gate still failing.
#[tokio::test]
async fn a_fix_run_on_a_red_pr_head_does_not_publish_its_red_gate() {
    const GATE: &str = "git grep -q HEALTHY -- README.md";
    let run = run_starter_launched(
        "address-review",
        |settings| {
            settings.worktree_strategy.test_command = Some(GATE.to_string());
        },
        seed_red_pr_head,
        |launch| {
            launch.origin = crate::domain::feature_origin::FeatureOrigin::Branch {
                base: "pr-head".to_string(),
            };
            launch.diff_base_branch = Some("main".to_string());
        },
    )
    .await
    .snapshot;

    let step = |id: &str| {
        run.steps
            .iter()
            .find(|s| s.step_id == id)
            .unwrap_or_else(|| panic!("no step {id} in {run:#?}"))
    };

    assert_eq!(run.terminal_status, "failed", "{run:#?}");
    let confirm = step("s-confirm");
    assert_eq!(confirm.status, "failed", "{run:#?}");
    assert!(
        confirm.error.as_deref().is_some_and(|e| e.contains(GATE)),
        "s-confirm must fail on the gate itself, not on the fixture: {run:#?}"
    );
    assert_ne!(step("s-finalize").status, "completed", "{run:#?}");
}

/// `main` green and `pr-head` red for `git grep -q HEALTHY -- README.md`,
/// both pushed to a local bare `origin` so a `Branch` origin can fetch them.
fn seed_red_pr_head(dir: &Path) {
    let parent = dir.parent().expect("repo dir has a parent");
    let origin = parent.join("starter-origin.git");
    std::fs::create_dir_all(&origin).expect("create origin dir");
    git_in(&origin, &["init", "--bare", "-b", "main"]);

    std::fs::create_dir_all(dir).expect("create repo dir");
    let git = |args: &[&str]| git_in(dir, args);
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# fixture\n\nHEALTHY\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
    let origin_url = origin.to_string_lossy().into_owned();
    git(&["remote", "add", "origin", &origin_url]);
    git(&["push", "origin", "main"]);

    git(&["checkout", "-b", "pr-head"]);
    std::fs::write(dir.join("README.md"), "# fixture\n").expect("break README");
    git(&["commit", "-am", "break the gate"]);
    git(&["push", "origin", "pr-head"]);
    git(&["checkout", "main"]);
}

/// A fix run launched with a workflow that carries the eager
/// `s-baseline-harness` node measures that node at the fork point the
/// subtraction reads, not at the pull request's head its worktree is cut from.
///
/// `main` is the fork point, and the gate is green there. Every later
/// harness-first pass on the red head then finds a record that covers the fork
/// point and names the gate under the same command, so the lazy fallback never
/// runs and the stored gate stays the node's.
///
/// Watched red with the node measuring its head again: it records `pr-head`'s
/// sha with the gate red, the subtraction's fork point is `main`, and the lazy
/// fallback replaces that record with its own measurement of `main` — so the
/// stored `base_sha` is the same, but its gate says `Fallback`.
#[tokio::test]
async fn a_fix_run_measures_its_baseline_node_at_the_fork_point() {
    let main_sha = std::sync::OnceLock::new();
    let run = run_starter_launched(
        "simple-task",
        |settings| {
            settings.worktree_strategy.test_command =
                Some("git grep -q HEALTHY -- README.md".to_string());
        },
        |dir| {
            seed_red_pr_head(dir);
            let out = std::process::Command::new("git")
                .args(["rev-parse", "main"])
                .current_dir(dir)
                .output()
                .expect("run git");
            let _ = main_sha.set(String::from_utf8_lossy(&out.stdout).trim().to_string());
        },
        |launch| {
            launch.origin = crate::domain::feature_origin::FeatureOrigin::Branch {
                base: "pr-head".to_string(),
            };
            launch.diff_base_branch = Some("main".to_string());
        },
    )
    .await;
    let snapshot = &run.snapshot;

    let node = snapshot
        .steps
        .iter()
        .find(|s| s.step_id == "s-baseline-harness")
        .unwrap_or_else(|| panic!("no baseline node in {snapshot:#?}"));
    assert_eq!(node.status, "completed", "{snapshot:#?}");

    let baseline = run
        .harness_baseline
        .as_ref()
        .unwrap_or_else(|| panic!("the node must write a baseline record: {snapshot:#?}"));
    assert_eq!(
        Some(&baseline.base_sha),
        main_sha.get(),
        "the record must name the fork point, not the head the fix branch was cut from"
    );
    let gate = baseline
        .harness("default")
        .expect("the project's test_command is measured under the default gate name");
    assert_eq!(
        gate.producer,
        crate::domain::harness_baseline::BaselineProducer::Node,
        "the node's measurement must be the one the subtraction read: {baseline:#?}"
    );
    assert!(
        gate.exit_ok,
        "the gate is green at the fork point: {baseline:#?}"
    );
    assert!(
        run.node_worktrees.is_empty(),
        "the node's fork-point checkout must be torn down: {:?}",
        run.node_worktrees
    );
}

/// A fix run's baseline node measures the pull request's target, and a target
/// that cannot prepare on this machine is no statement about the head the run
/// validates. The node completes with no base to subtract and the run goes on.
///
/// `prepare_command` passes only where `pr-head` added its marker, so it fails
/// at the fork point and succeeds everywhere the run itself works.
///
/// Watched red with the node reading `baseline_node_verdict` directly: the
/// empty fork-point measurement is `Unmeasurable`, and the node ends
/// `Environmental` before any agent runs — refusing to fix a pull request
/// because `main` cannot install here.
#[tokio::test]
async fn a_fix_run_whose_target_cannot_prepare_still_runs() {
    let main_sha = std::sync::OnceLock::new();
    let run = run_starter_launched(
        "simple-task",
        |settings| {
            settings.worktree_strategy.prepare_command =
                Some("git grep -q PREPARED -- README.md".to_string());
        },
        |dir| {
            seed_prepared_pr_head(dir);
            let out = std::process::Command::new("git")
                .args(["rev-parse", "main"])
                .current_dir(dir)
                .output()
                .expect("run git");
            let _ = main_sha.set(String::from_utf8_lossy(&out.stdout).trim().to_string());
        },
        |launch| {
            launch.origin = crate::domain::feature_origin::FeatureOrigin::Branch {
                base: "pr-head".to_string(),
            };
            launch.diff_base_branch = Some("main".to_string());
        },
    )
    .await;
    let snapshot = &run.snapshot;

    let node = snapshot
        .steps
        .iter()
        .find(|s| s.step_id == "s-baseline-harness")
        .unwrap_or_else(|| panic!("no baseline node in {snapshot:#?}"));
    assert_eq!(node.status, "completed", "{snapshot:#?}");
    assert_ne!(snapshot.terminal_status, "failed", "{snapshot:#?}");
    assert!(
        run.environment_not_ready.is_empty(),
        "the target's health is not this machine's verdict on the head: {:?}",
        run.environment_not_ready
    );
    assert!(
        run.harness_baseline
            .as_ref()
            .is_none_or(|b| Some(&b.base_sha) != main_sha.get()),
        "nothing was measured at the fork point, so no record may claim to cover it: {:#?}",
        run.harness_baseline
    );
    assert!(
        snapshot
            .artifacts
            .iter()
            .any(|(step, _, body)| step == "s-baseline-harness" && body.contains("fork point")),
        "the node's Output tab must say why there is no base: {snapshot:#?}"
    );
    assert!(
        run.node_worktrees.is_empty(),
        "the node's fork-point checkout must be torn down: {:?}",
        run.node_worktrees
    );
}

/// The required filter lives only in `main`'s attributes, so Git can provision
/// the head worktree but cannot provision the detached fork-point checkout.
#[tokio::test]
async fn a_fork_point_checkout_failure_measures_the_head_in_place() {
    let run = run_starter_launched(
        "simple-task",
        |settings| {
            settings.worktree_strategy.prepare_command =
                Some("git grep -q PREPARED -- README.md".to_string());
        },
        seed_fork_point_with_unavailable_filter,
        |launch| {
            launch.origin = crate::domain::feature_origin::FeatureOrigin::Branch {
                base: "pr-head".to_string(),
            };
            launch.diff_base_branch = Some("main".to_string());
        },
    )
    .await;
    let snapshot = &run.snapshot;
    let node = snapshot
        .steps
        .iter()
        .find(|s| s.step_id == "s-baseline-harness")
        .unwrap_or_else(|| panic!("no baseline node in {snapshot:#?}"));

    assert_eq!(node.status, "failed", "{snapshot:#?}");
    assert_eq!(snapshot.terminal_status, "failed", "{snapshot:#?}");
    assert!(
        node.error.as_deref().is_some_and(|e| e.contains("prepare")),
        "the in-place measurement must keep its terminal answer: {snapshot:#?}"
    );
    assert!(run.harness_baseline.is_none(), "{snapshot:#?}");
    assert!(run.node_worktrees.is_empty(), "{:?}", run.node_worktrees);
}

fn seed_fork_point_with_unavailable_filter(dir: &Path) {
    let parent = dir.parent().expect("repo dir has a parent");
    let origin = parent.join("starter-origin.git");
    std::fs::create_dir_all(&origin).expect("create origin dir");
    git_in(&origin, &["init", "--bare", "-b", "main"]);

    std::fs::create_dir_all(dir).expect("create repo dir");
    let git = |args: &[&str]| git_in(dir, args);
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# fixture\n").expect("seed README");
    std::fs::write(dir.join(".gitattributes"), "README.md filter=unavailable\n")
        .expect("seed filter attribute");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
    let origin_url = origin.to_string_lossy().into_owned();
    git(&["remote", "add", "origin", &origin_url]);
    git(&["push", "origin", "main"]);

    git(&["checkout", "-b", "pr-head"]);
    std::fs::write(dir.join(".gitattributes"), "").expect("remove filter attribute at head");
    git(&["add", "-A"]);
    git(&["commit", "-m", "make head check out without filter"]);
    git(&["push", "origin", "pr-head"]);
    git(&["checkout", "main"]);
    git(&[
        "config",
        "filter.unavailable.smudge",
        "git rev-parse --verify refs/heads/missing-filter-ref",
    ]);
    git(&["config", "filter.unavailable.required", "true"]);
}

/// `main` without and `pr-head` with a `PREPARED` marker in `README.md`, both
/// pushed to a local bare `origin` so a `Branch` origin can fetch them.
fn seed_prepared_pr_head(dir: &Path) {
    let parent = dir.parent().expect("repo dir has a parent");
    let origin = parent.join("starter-origin.git");
    std::fs::create_dir_all(&origin).expect("create origin dir");
    git_in(&origin, &["init", "--bare", "-b", "main"]);

    std::fs::create_dir_all(dir).expect("create repo dir");
    let git = |args: &[&str]| git_in(dir, args);
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "demeteo@local"]);
    git(&["config", "user.name", "demeteo"]);
    std::fs::write(dir.join("README.md"), "# fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
    let origin_url = origin.to_string_lossy().into_owned();
    git(&["remote", "add", "origin", &origin_url]);
    git(&["push", "origin", "main"]);

    git(&["checkout", "-b", "pr-head"]);
    std::fs::write(dir.join("README.md"), "# fixture\n\nPREPARED\n").expect("mark README");
    git(&["commit", "-am", "make the checkout preparable"]);
    git(&["push", "origin", "pr-head"]);
    git(&["checkout", "main"]);
}
