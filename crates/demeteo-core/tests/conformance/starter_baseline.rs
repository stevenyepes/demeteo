//! Baseline behavioral harness for the seven bundled starter workflows
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

/// Seed a real local git repo at the project's expected repo dir so
/// `bootstrap_project` skips its (network) clone — the same "already
/// cloned" shortcut the topology and triage gates use.
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
    ctx.projects.save_settings(settings).expect("save settings");

    init_local_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow = augment_starter(load_starter(name));
    let workflow_id =
        workflows::create_from_json(&ctx.workflows, &workflow).expect("ingest starter");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            &format!("Baseline {name}"),
            "Deterministic starter-baseline run under the stub agent.",
            Some("stub"),
            // model / effort / commit_artifacts / loop_iterations /
            // max_budget_usd: inherit.
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

    let terminal_status = poll_terminal_approving_gates(&ctx, &feature.id).await;

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
    StarterSnapshot {
        starter: name.to_string(),
        terminal_status,
        steps: step_snaps,
        artifacts,
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
async fn starter_baseline_standard_feature_pipeline() {
    assert_starter_baseline("standard-feature-pipeline").await;
}
