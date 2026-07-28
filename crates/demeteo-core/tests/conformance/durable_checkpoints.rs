//! Durable-checkpoint crash-resume fixture (P1.9, `docs/TASKS_DAG_WORKFLOWS.md`).
//!
//! The V32 tables exist so that a driver restart resumes a `sequence`
//! step **from the exact task**, not the step head. This suite proves it
//! end-to-end with the deterministic stub agent:
//!
//! 1. A plan → sequence feature runs to completion once, establishing
//!    that both stub tasks execute on a pristine run (the control).
//! 2. The run is then rewound to look exactly like a driver that died
//!    mid-sequence after checkpointing its landed prefix: the sequence
//!    step row goes to `interrupted`, the feature to `failed`, and the
//!    V32 checkpoint records `stub-task-1` as landed.
//! 3. A **second app life** (`build_core_context` over the same data
//!    dir — a genuine restart: fresh executor, fresh driver registry,
//!    no in-memory state) retries the step. With the old in-memory maps
//!    this re-ran the whole list; with V32 it must run only
//!    `stub-task-2`.
//!
//! The assertion reads `subtask_runs` — one row per (task, attempt) —
//! through a direct connection to the same SQLite file, so the observed
//! evidence is the engine's own audit trail.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::adapters::agent::stub_runtime::STUB_AGENT_ENV;
use crate::application::{bootstrap, projects, workflows};
use crate::composition::{build_core_context, CoreConfig, ExecutionMode};
use crate::domain::ids::{FeatureId, ProviderId};
use crate::domain::models::ProviderInstance;
use crate::paths;
use crate::ports::db::FeaturePatch;
use crate::ports::db::StepExecutionPatch;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::state::AppContext;

const REPO_PATH: &str = "demeteo/durable-checkpoints";
const PROVIDER_ID: &str = "durable-checkpoints-provider";

struct NoopNotif;
impl NotificationPort for NoopNotif {
    fn emit(&self, _event: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

/// Plan step writes the stub's deterministic two-task list
/// (`stub-task-1`, `stub-task-2`); the sequence step consumes it via
/// `task_list_from`.
fn plan_then_sequence_workflow() -> serde_json::Value {
    serde_json::json!({
        "name": "Durable Checkpoint Resume",
        "description": "Plan + sequence pair for the P1.9 crash-resume gate.",
        "steps": [
            {
                "id": "s-plan",
                "kind": "agent",
                "title": "Plan",
                "agent_kind": "stub",
                "prompt_template": "Write the ticket list.\n@stub-write artifacts/task-list.json\n",
                "capability": "artifacts",
                "artifacts": [
                    {
                        "name": "task-list",
                        "capture": { "kind": "last_write_to", "path": "artifacts/task-list.json" },
                        "mode": "full"
                    }
                ]
            },
            {
                "id": "s-impl",
                "kind": "sequence",
                "title": "Implement",
                "agent_kind": "stub",
                "task_list_from": "s-plan",
                "prompt_template": "Implement the task.\n@stub-write artifacts/task-notes.md\n",
                "artifacts": [
                    {
                        "name": "implemented-files",
                        "capture": { "kind": "all_writes" },
                        "mode": "summary_only"
                    }
                ]
            }
        ]
    })
}

/// Same fixture repo the starter-baseline harness uses: a README-only
/// local git repo at the engine's canonical local target dir, so
/// bootstrap adopts it instead of trying (and failing headless) to
/// clone through the keyring.
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
    std::fs::write(dir.join("README.md"), "# durable checkpoint fixture\n").expect("seed README");
    git(&["add", "-A"]);
    git(&["commit", "-m", "seed"]);
}

async fn poll_terminal(ctx: &AppContext, feature_id: &FeatureId) -> String {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let status = ctx
            .features
            .get(feature_id)
            .ok()
            .flatten()
            .map(|f| f.status)
            .unwrap_or_default();
        // `awaiting_mr` is the local-run terminal parking state when no
        // publisher is wired (same treatment as the starter baseline).
        if matches!(
            status.as_str(),
            "completed" | "failed" | "cancelled" | "awaiting_mr"
        ) {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "feature did not reach a terminal status (last: {status})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `(subtask_id, status)` rows for the feature, in insertion order, read
/// through a direct connection to the app's SQLite file.
fn subtask_runs(app_data_dir: &Path, feature_id: &FeatureId) -> Vec<(String, String)> {
    let conn = rusqlite::Connection::open(app_data_dir.join("demeteo.db")).expect("open db file");
    let mut stmt = conn
        .prepare(
            "SELECT subtask_id, status FROM subtask_runs
             WHERE feature_id = ?1 ORDER BY rowid",
        )
        .expect("prepare");
    let rows = stmt
        .query_map(rusqlite::params![feature_id.0], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .expect("query")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect");
    rows
}

/// Run a `git` command in `dir`, asserting success.
fn git_in(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {:?} in {} failed: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// The same sequence step, plus a declared artifact **no stub task ever
/// produces**.
///
/// That is the cheapest way to reach a failure the step raises *after* its
/// whole task list has landed — step 3b's `never_produced` check — which is
/// the shape that matters here: every task committed and checkpointed, and
/// then the attempt is deliberately thrown away. The verifier verdict and
/// the "branch carries no changes" path reach the same rollback; this one
/// needs no verifier config to drive.
fn plan_then_sequence_with_an_unproducible_artifact() -> serde_json::Value {
    let mut wf = plan_then_sequence_workflow();
    wf["steps"][1]["artifacts"]
        .as_array_mut()
        .expect("sequence step declares artifacts")
        .push(serde_json::json!({
            "name": "verification-report",
            "capture": { "kind": "by_name", "name": "verification-report" },
            "mode": "full"
        }));
    wf["name"] = serde_json::json!("Durable Checkpoint Rollback");
    wf
}

/// Bootstrap a project, run `workflow` once, and hand back the terminal
/// status for the caller to judge — some of these fixtures are supposed to
/// fail.
async fn start_run(
    tag: &str,
    workflow: &serde_json::Value,
) -> (std::path::PathBuf, AppContext, String, FeatureId, String) {
    std::env::set_var(STUB_AGENT_ENV, "1");
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-durable-ckpt-{}-{}",
        tag,
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
            name: "durable-checkpoints".to_string(),
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
    settings.worktree_strategy.test_command = Some("true".to_string());
    ctx.projects.save_settings(settings).expect("save settings");

    init_local_repo(&ctx.workspace_dir, project.id.as_str(), REPO_PATH);
    bootstrap::bootstrap_project(&ctx, project.id.0.clone())
        .await
        .expect("bootstrap project");

    let workflow_id =
        workflows::create_from_json(&ctx.workflows, workflow).expect("ingest workflow");

    let feature = ctx
        .executor
        .feature_start(
            None,
            project.id.as_str(),
            workflow_id.as_str(),
            "Durable checkpoint resume",
            "Run the stub plan + sequence pair to completion, then rewind.",
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
    (tmp, ctx, project.id.0.clone(), feature.id, status)
}

/// Everything the resume tests need: a bootstrapped project running the
/// plan + sequence workflow once to completion, proving on the way that a
/// pristine run executes both stub tasks.
///
/// Returned as owned handles rather than a fixture struct because each
/// test rewinds the same state differently — one to the V32 shape (prefix
/// merged), one to the V35 shape (prefix stranded on the step branch).
async fn control_run(tag: &str) -> (std::path::PathBuf, AppContext, String, FeatureId) {
    let (tmp, ctx, project_id, feature_id, status) =
        start_run(tag, &plan_then_sequence_workflow()).await;

    // ── Life 1 (control): a pristine run executes both stub tasks. ──
    let step_debug: Vec<String> = ctx
        .features
        .steps_for_feature(&feature_id)
        .unwrap_or_default()
        .iter()
        .map(|s| format!("{}={} err={:?}", s.step_id.0, s.status, s.error_message))
        .collect();
    assert_eq!(
        status, "awaiting_mr",
        "control run failed; steps: {step_debug:#?}"
    );
    let life1 = subtask_runs(&tmp, &feature_id);
    let life1_tasks: Vec<&str> = life1.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        life1_tasks.iter().any(|t| t.contains("stub-task-1"))
            && life1_tasks.iter().any(|t| t.contains("stub-task-2")),
        "control run must execute both stub tasks; ran: {life1_tasks:?}"
    );

    (tmp, ctx, project_id, feature_id)
}

/// The P1.9 exit test: a fresh driver life resumes a checkpointed
/// sequence step from the exact task, not the step head.
#[tokio::test]
async fn restart_resumes_sequence_from_the_exact_task() {
    let (tmp, ctx, _project_id, feature_id) = control_run("merged").await;

    // ── Rewind: forge the exact on-disk state a driver killed
    // mid-sequence leaves behind — sequence step interrupted, feature
    // failed, and the V32 checkpoint recording task 1's prefix as
    // already merged. ──
    let steps = ctx.features.steps_for_feature(&feature_id).expect("steps");
    let impl_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    ctx.features
        .step_update(
            &impl_step.id,
            &StepExecutionPatch {
                status: Some("interrupted".to_string()),
                error_message: Some(None),
                ..Default::default()
            },
        )
        .expect("reset sequence step");
    ctx.features
        .update(
            &feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("reset feature status");
    ctx.features
        .sequence_checkpoint_record(
            &feature_id,
            "s-impl",
            &["stub-task-1".to_string()],
            // No anchor: a V32-shaped row, which means "the prefix is
            // already merged to the feature branch" — the state this test
            // forges. The V35 crash shape (prefix on the step branch, with
            // an anchor to restore it from) is covered separately.
            None,
            None,
            paths::now_ms(),
        )
        .expect("seed checkpoint");
    let baseline_rows = subtask_runs(&tmp, &feature_id).len();

    // ── Life 2: a genuine app restart — a second composition over the
    // same data dir, holding none of the first life's memory — retries
    // the interrupted step. ──
    let ctx2 = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    ctx2.executor
        .step_retry(impl_step.id.0.as_str(), None, None, None)
        .await
        .expect("retry the interrupted sequence step");
    assert_eq!(poll_terminal(&ctx2, &feature_id).await, "awaiting_mr");

    let life2: Vec<(String, String)> = subtask_runs(&tmp, &feature_id)
        .into_iter()
        .skip(baseline_rows)
        .collect();
    assert!(
        !life2.is_empty(),
        "the resumed driver must have run the remainder of the list"
    );
    assert!(
        life2.iter().all(|(id, _)| !id.contains("stub-task-1")),
        "resume must skip the checkpointed task, not re-run the step head; life 2 ran: {life2:?}"
    );
    assert!(
        life2.iter().any(|(id, _)| id.contains("stub-task-2")),
        "resume must run the exact remaining task; life 2 ran: {life2:?}"
    );

    // The completed step spends its checkpoint — a stale skip-list must
    // not exempt tasks from a future full re-run.
    assert!(
        ctx2.features
            .sequence_checkpoint_get(&feature_id, "s-impl")
            .expect("read checkpoint")
            .is_empty(),
        "completing the step must clear the durable checkpoint"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// The V35 exit test: an *interrupted* attempt's committed work survives
/// the restart and ends up on the feature branch.
///
/// This is the shape V32 could not express. A killed process never
/// reaches the mid-list failure path, so nothing merges: the finished
/// tasks are left committed on the step branch alone. The next attempt
/// then re-provisions that worktree — `worktree remove` + `rm -rf` +
/// `worktree add`, which resets the step branch back to the feature
/// branch — so unless something says otherwise, hours of paid work are
/// discarded and the list re-runs from task one.
///
/// The forge below reproduces that state exactly, including the part that
/// makes it dangerous: the step branch is *deleted*, and only the
/// checkpoint ref still holds the commit. The assertion is not just that
/// the finished task is skipped (a skip that dropped its work would be
/// worse than re-running it) but that its file is on the feature branch
/// when the step completes.
#[tokio::test]
async fn restart_restores_an_interrupted_attempt_committed_work() {
    let (tmp, ctx, project_id, feature_id) = control_run("stranded").await;

    let repo_dir = paths::repo_target_dir_local(&ctx.workspace_dir, &project_id, REPO_PATH);
    let branch = format!(
        "{}{}",
        crate::adapters::step_executor::setup::fetch_default_settings()
            .worktree_strategy
            .branch_prefix,
        feature_id.0
    );

    // ── Forge the stranded prefix: a commit made on a step branch that
    // was never merged, whose branch is then deleted — exactly what
    // `cleanup_subtask_worktree` leaves behind — with only the checkpoint
    // ref keeping it reachable. ──
    let forge_wt = tmp.join("forge-wt");
    let forge_branch = format!("{}_subtask_forge", branch);
    git_in(
        &repo_dir,
        &[
            "worktree",
            "add",
            forge_wt.to_str().unwrap(),
            "-b",
            &forge_branch,
            &branch,
        ],
    );
    std::fs::write(
        forge_wt.join("stub-task-1-work.txt"),
        "work the interrupted attempt committed and was paid for\n",
    )
    .expect("write forged task output");
    git_in(&forge_wt, &["add", "-A"]);
    git_in(&forge_wt, &["commit", "-m", "chore: stub-task-1"]);
    let anchor = git_in(&forge_wt, &["rev-parse", "HEAD"]);

    let checkpoint_ref = format!("refs/demeteo/seq/{}/{}", feature_id.0, "s-impl");
    git_in(&repo_dir, &["update-ref", &checkpoint_ref, &anchor]);
    git_in(
        &repo_dir,
        &["worktree", "remove", "--force", forge_wt.to_str().unwrap()],
    );
    git_in(&repo_dir, &["branch", "-D", &forge_branch]);

    // The commit is now unreachable from any branch — only the checkpoint
    // ref holds it. That is the whole point of writing the ref.
    assert!(
        !git_in(&repo_dir, &["branch", "--contains", &anchor])
            .lines()
            .any(|l| l.trim_start_matches('*').trim() == branch),
        "the forged prefix must NOT be on the feature branch; that is the V32 shape"
    );

    let steps = ctx.features.steps_for_feature(&feature_id).expect("steps");
    let impl_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    ctx.features
        .step_update(
            &impl_step.id,
            &StepExecutionPatch {
                status: Some("interrupted".to_string()),
                error_message: Some(None),
                ..Default::default()
            },
        )
        .expect("reset sequence step");
    ctx.features
        .update(
            &feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("reset feature status");
    ctx.features
        .sequence_checkpoint_record(
            &feature_id,
            "s-impl",
            &["stub-task-1".to_string()],
            Some(anchor.as_str()),
            None,
            paths::now_ms(),
        )
        .expect("seed checkpoint");
    let baseline_rows = subtask_runs(&tmp, &feature_id).len();

    // ── Life 2: a genuine app restart retries the interrupted step. ──
    let ctx2 = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    ctx2.executor
        .step_retry(impl_step.id.0.as_str(), None, None, None)
        .await
        .expect("retry the interrupted sequence step");
    let status = poll_terminal(&ctx2, &feature_id).await;
    let step_debug: Vec<String> = ctx2
        .features
        .steps_for_feature(&feature_id)
        .unwrap_or_default()
        .iter()
        .map(|s| format!("{}={} err={:?}", s.step_id.0, s.status, s.error_message))
        .collect();
    assert_eq!(
        status, "awaiting_mr",
        "resume failed; steps: {step_debug:#?}"
    );

    let life2: Vec<(String, String)> = subtask_runs(&tmp, &feature_id)
        .into_iter()
        .skip(baseline_rows)
        .collect();
    assert!(
        life2.iter().all(|(id, _)| !id.contains("stub-task-1")),
        "the interrupted attempt's finished task must not re-run; life 2 ran: {life2:?}"
    );
    assert!(
        life2.iter().any(|(id, _)| id.contains("stub-task-2")),
        "resume must run the remaining task; life 2 ran: {life2:?}"
    );

    // The assertion that matters: skipping the task kept its work rather
    // than dropping it. A resume that skips *and* discards is the failure
    // mode this whole mechanism exists to prevent.
    let tree = git_in(&repo_dir, &["ls-tree", "-r", "--name-only", &branch]);
    assert!(
        tree.lines().any(|f| f == "stub-task-1-work.txt"),
        "the interrupted attempt's committed work must reach the feature branch; \
         branch holds: {tree}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// Run a `git` command in `dir`, reporting only whether it succeeded.
fn git_succeeds(dir: &Path, args: &[&str]) -> bool {
    std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run git")
        .status
        .success()
}

/// A rollback has to take the checkpoint with it.
///
/// Since V35 the checkpoint row does not merely say *skip these ids* — it
/// names a commit the next attempt will `reset --hard` onto. So an attempt
/// that lands its whole task list and is then thrown away (here: a declared
/// artifact no task produced; identically, a verifier verdict) must not
/// leave that row standing. If it does, the retry's `resolve_checkpoint_resume`
/// sees an anchor that is not an ancestor of the rolled-back branch, reads
/// it as the crash shape, and restores exactly the commits the rollback
/// existed to discard — a rejected implementation quietly reinstated.
///
/// Asserts on the row *and* the ref, because the two are what the next
/// attempt reads; the branch state alone would look identical either way.
#[tokio::test]
async fn a_rollback_after_the_task_list_lands_rewinds_the_checkpoint() {
    let (tmp, ctx, project_id, feature_id, status) = start_run(
        "rollback",
        &plan_then_sequence_with_an_unproducible_artifact(),
    )
    .await;

    let steps = ctx.features.steps_for_feature(&feature_id).expect("steps");
    let impl_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    let step_debug: Vec<String> = steps
        .iter()
        .map(|s| format!("{}={} err={:?}", s.step_id.0, s.status, s.error_message))
        .collect();

    // Precondition: the fixture failed the way it was meant to — after the
    // task list ran, not before it. A step that never reached the task loop
    // would pass every assertion below for the wrong reason.
    assert_eq!(status, "failed", "steps: {step_debug:#?}");
    assert!(
        impl_step
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("verification-report"),
        "the sequence step must have failed on its unproduced artifact, not earlier; \
         steps: {step_debug:#?}"
    );
    let ran = subtask_runs(&tmp, &feature_id);
    let ran_tasks: Vec<&str> = ran.iter().map(|(id, _)| id.as_str()).collect();
    assert!(
        ran_tasks.iter().any(|t| t.contains("stub-task-1"))
            && ran_tasks.iter().any(|t| t.contains("stub-task-2")),
        "the whole task list must have landed before the rollback; ran: {ran_tasks:?}"
    );

    // The assertion that matters: nothing is left telling the next attempt
    // to restore what was just discarded.
    let checkpoint = ctx
        .features
        .sequence_checkpoint_get(&feature_id, "s-impl")
        .expect("read checkpoint");
    assert!(
        checkpoint.is_empty(),
        "a rolled-back attempt must not leave landed task ids behind; found {:?}",
        checkpoint.landed_task_ids
    );
    assert_eq!(
        checkpoint.anchor_sha, None,
        "a rolled-back attempt must not leave an anchor for the retry to reset onto"
    );

    let repo_dir = paths::repo_target_dir_local(&ctx.workspace_dir, &project_id, REPO_PATH);
    let checkpoint_ref = format!("refs/demeteo/seq/{}/{}", feature_id.0, "s-impl");
    assert!(
        !git_succeeds(&repo_dir, &["rev-parse", "--verify", &checkpoint_ref]),
        "the rollback must unpin the discarded prefix; {checkpoint_ref} still resolves"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// A resume must still read its task list from the artifact.
///
/// A checkpoint speaks task ids, so an attempt resuming against one has to
/// speak the same ids — which is why a *planner*-sourced step prefers its
/// cached plan (a fresh decomposition invents new ids and the checkpoint
/// would match nothing). A `task_list_from` step needs no such rescue: the
/// upstream artifact is the id's source and is stable across a re-read.
///
/// Extending the cache preference to it would cost something real, and this
/// is that cost: the human gate exists so a redirect can *revise* the task
/// list, and a step that answers from cache would run the superseded one
/// with nothing in the log to say it had. Here the revision drops
/// `stub-task-2` and adds `stub-task-3`; a resume reading the cache runs
/// task 2, a resume reading the artifact runs task 3.
#[tokio::test]
async fn a_resume_re_reads_a_revised_task_list_artifact() {
    let (tmp, ctx, _project_id, feature_id) = control_run("revised").await;

    let steps = ctx.features.steps_for_feature(&feature_id).expect("steps");
    let plan_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-plan")
        .expect("plan step exists");
    let impl_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");

    // ── The gate redirect: rewrite the task-list artifact in place, exactly
    // as a re-run of the planning step would have. ──
    let task_list_ref = plan_step
        .artifact_paths
        .iter()
        .find(|r| r.to_lowercase().contains("task-list"))
        .expect("the plan step wrote a task-list artifact");
    std::fs::write(
        task_list_ref,
        serde_json::json!({
            "tasks": [
                {
                    "id": "stub-task-1",
                    "title": "Already landed",
                    "description": "Kept so the checkpoint still matches something.",
                    "files": ["stub-task-1-work.txt"]
                },
                {
                    "id": "stub-task-3",
                    "title": "Added by the redirect",
                    "description": "Only a resume that re-reads the artifact will run this.",
                    "files": ["stub-task-3-work.txt"]
                }
            ]
        })
        .to_string(),
    )
    .expect("revise the task list artifact");

    // ── Rewind to a V32-shaped checkpoint: task 1 landed and already
    // merged, so there is nothing to restore and the resume turns purely on
    // which task list it reads. ──
    ctx.features
        .step_update(
            &impl_step.id,
            &StepExecutionPatch {
                status: Some("interrupted".to_string()),
                error_message: Some(None),
                ..Default::default()
            },
        )
        .expect("reset sequence step");
    ctx.features
        .update(
            &feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("reset feature status");
    ctx.features
        .sequence_checkpoint_record(
            &feature_id,
            "s-impl",
            &["stub-task-1".to_string()],
            None,
            None,
            paths::now_ms(),
        )
        .expect("seed checkpoint");
    let baseline_rows = subtask_runs(&tmp, &feature_id).len();

    let ctx2 = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    ctx2.executor
        .step_retry(impl_step.id.0.as_str(), None, None, None)
        .await
        .expect("retry the interrupted sequence step");
    let status = poll_terminal(&ctx2, &feature_id).await;
    let step_debug: Vec<String> = ctx2
        .features
        .steps_for_feature(&feature_id)
        .unwrap_or_default()
        .iter()
        .map(|s| format!("{}={} err={:?}", s.step_id.0, s.status, s.error_message))
        .collect();
    assert_eq!(
        status, "awaiting_mr",
        "resume failed; steps: {step_debug:#?}"
    );

    let life2: Vec<(String, String)> = subtask_runs(&tmp, &feature_id)
        .into_iter()
        .skip(baseline_rows)
        .collect();
    assert!(
        life2.iter().any(|(id, _)| id.contains("stub-task-3")),
        "the resume must run the task the redirect added; life 2 ran: {life2:?}"
    );
    assert!(
        life2.iter().all(|(id, _)| !id.contains("stub-task-2")),
        "the resume must not run the task the redirect removed — that is the cached plan \
         answering; life 2 ran: {life2:?}"
    );
    assert!(
        life2.iter().all(|(id, _)| !id.contains("stub-task-1")),
        "the checkpoint must still skip the landed task; life 2 ran: {life2:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// A kill *after* the last task commits must cost the merge, not the list.
///
/// This is the window the per-task checkpoint opens and, until the
/// `apply_landed_checkpoint` fallback was retired, could not exploit. From
/// the moment the last task lands until `clear_sequence_checkpoint` runs at
/// step completion, the row names **every** id in the plan — and that span
/// contains the declared-artifact check, the verifier's agent pass, and the
/// final merge. The verifier alone can be minutes.
///
/// The old fallback read "every id matches" as a stale row and put the whole
/// plan back, which under V32 was right (its only writer left a task
/// unlanded by construction) and under V35 re-pays for a 25-task step killed
/// during verification. Same bug the PR set out to fix, displaced from
/// "interrupted at task 21" to "interrupted after task 25".
///
/// So: zero tasks may run, and the work must still reach the feature branch.
#[tokio::test]
async fn a_kill_after_the_last_task_resumes_without_re_running_any() {
    let (tmp, ctx, project_id, feature_id) = control_run("merge-window").await;

    let repo_dir = paths::repo_target_dir_local(&ctx.workspace_dir, &project_id, REPO_PATH);
    let branch = format!(
        "{}{}",
        crate::adapters::step_executor::setup::fetch_default_settings()
            .worktree_strategy
            .branch_prefix,
        feature_id.0
    );

    // ── Forge the merge-window crash: both tasks committed on a step branch
    // that never merged, the branch then gone, the commit held only by the
    // checkpoint ref. ──
    let forge_wt = tmp.join("forge-wt");
    let forge_branch = format!("{}_subtask_forge", branch);
    git_in(
        &repo_dir,
        &[
            "worktree",
            "add",
            forge_wt.to_str().unwrap(),
            "-b",
            &forge_branch,
            &branch,
        ],
    );
    std::fs::write(
        forge_wt.join("merge-window-work.txt"),
        "both tasks committed; the process died before the merge\n",
    )
    .expect("write forged task output");
    git_in(&forge_wt, &["add", "-A"]);
    git_in(&forge_wt, &["commit", "-m", "chore: the whole task list"]);
    let anchor = git_in(&forge_wt, &["rev-parse", "HEAD"]);

    let checkpoint_ref = format!("refs/demeteo/seq/{}/{}", feature_id.0, "s-impl");
    git_in(&repo_dir, &["update-ref", &checkpoint_ref, &anchor]);
    git_in(
        &repo_dir,
        &["worktree", "remove", "--force", forge_wt.to_str().unwrap()],
    );
    git_in(&repo_dir, &["branch", "-D", &forge_branch]);

    let steps = ctx.features.steps_for_feature(&feature_id).expect("steps");
    let impl_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    ctx.features
        .step_update(
            &impl_step.id,
            &StepExecutionPatch {
                status: Some("interrupted".to_string()),
                error_message: Some(None),
                ..Default::default()
            },
        )
        .expect("reset sequence step");
    ctx.features
        .update(
            &feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        )
        .expect("reset feature status");
    // The whole plan, checkpointed — what the task loop leaves behind once
    // the last task commits.
    ctx.features
        .sequence_checkpoint_record(
            &feature_id,
            "s-impl",
            &["stub-task-1".to_string(), "stub-task-2".to_string()],
            Some(anchor.as_str()),
            None,
            paths::now_ms(),
        )
        .expect("seed checkpoint");
    let baseline_rows = subtask_runs(&tmp, &feature_id).len();

    let ctx2 = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    ctx2.executor
        .step_retry(impl_step.id.0.as_str(), None, None, None)
        .await
        .expect("retry the interrupted sequence step");
    let status = poll_terminal(&ctx2, &feature_id).await;
    let step_debug: Vec<String> = ctx2
        .features
        .steps_for_feature(&feature_id)
        .unwrap_or_default()
        .iter()
        .map(|s| format!("{}={} err={:?}", s.step_id.0, s.status, s.error_message))
        .collect();
    assert_eq!(
        status, "awaiting_mr",
        "resume failed; steps: {step_debug:#?}"
    );

    let life2: Vec<(String, String)> = subtask_runs(&tmp, &feature_id)
        .into_iter()
        .skip(baseline_rows)
        .collect();
    assert!(
        life2.is_empty(),
        "a fully-landed checkpoint must run no tasks at all; life 2 ran: {life2:?}"
    );

    // Skipping is only right if the work survives — the same assertion the
    // partial-resume test makes, for the case where the whole list landed.
    let tree = git_in(&repo_dir, &["ls-tree", "-r", "--name-only", &branch]);
    assert!(
        tree.lines().any(|f| f == "merge-window-work.txt"),
        "the killed attempt's committed work must reach the feature branch; \
         branch holds: {tree}"
    );

    // The step still has to hand its declared deliverable downstream, even
    // though no task ran to re-produce it.
    let impl_after = ctx2
        .features
        .steps_for_feature(&feature_id)
        .expect("steps")
        .into_iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    assert!(
        !impl_after.artifact_paths.is_empty(),
        "a resumed step must still carry artifacts; the declared deliverables were \
         produced by the attempt that landed the tasks"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}

/// A resumed step is judged on the same evidence as a fresh one, and a
/// verdict against it must let the step move.
///
/// Two failures met here. **The gate**: skipping `never_produced` whenever
/// no task ran meant a step whose declared deliverable was never written
/// completed green on resume while a fresh run of the same workflow fails —
/// the error then surfacing downstream as something unrelated. V36's
/// `produced_json` lets the resume answer instead of being exempted.
///
/// **Termination**: a zero-task attempt rolled back with `RewindTo` writes
/// the identical row back, so the next attempt resumes the same way, runs no
/// agents, and is judged the same way — forever, with the feedback never
/// reaching anything that could act on it. A verdict on a zero-task attempt
/// discards the checkpoint instead, which puts the tasks back in the plan.
#[tokio::test]
async fn a_resumed_step_still_fails_its_missing_deliverable_and_can_recover() {
    let (tmp, ctx, project_id, feature_id, _status) = start_run(
        "resumed-gate",
        &plan_then_sequence_with_an_unproducible_artifact(),
    )
    .await;

    let repo_dir = paths::repo_target_dir_local(&ctx.workspace_dir, &project_id, REPO_PATH);
    let branch = format!(
        "{}{}",
        crate::adapters::step_executor::setup::fetch_default_settings()
            .worktree_strategy
            .branch_prefix,
        feature_id.0
    );

    // ── Forge a fully-landed checkpoint whose payload records what the
    // tasks actually produced: `implemented-files`, and *not* the declared
    // `verification-report`. That is the honest shape of a kill after the
    // last task committed, in a workflow whose deliverable was never
    // written. ──
    let forge_wt = tmp.join("forge-wt");
    let forge_branch = format!("{}_subtask_forge", branch);
    git_in(
        &repo_dir,
        &[
            "worktree",
            "add",
            forge_wt.to_str().unwrap(),
            "-b",
            &forge_branch,
            &branch,
        ],
    );
    std::fs::write(forge_wt.join("resumed-gate-work.txt"), "landed\n").expect("write forged work");
    git_in(&forge_wt, &["add", "-A"]);
    git_in(&forge_wt, &["commit", "-m", "chore: the whole task list"]);
    let anchor = git_in(&forge_wt, &["rev-parse", "HEAD"]);
    git_in(
        &repo_dir,
        &[
            "update-ref",
            &format!("refs/demeteo/seq/{}/{}", feature_id.0, "s-impl"),
            &anchor,
        ],
    );
    git_in(
        &repo_dir,
        &["worktree", "remove", "--force", forge_wt.to_str().unwrap()],
    );
    git_in(&repo_dir, &["branch", "-D", &forge_branch]);

    let steps = ctx.features.steps_for_feature(&feature_id).expect("steps");
    let impl_step = steps
        .iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    let reset_for_retry = |ctx: &AppContext| {
        ctx.features
            .step_update(
                &impl_step.id,
                &StepExecutionPatch {
                    status: Some("interrupted".to_string()),
                    error_message: Some(None),
                    ..Default::default()
                },
            )
            .expect("reset sequence step");
        ctx.features
            .update(
                &feature_id,
                &FeaturePatch {
                    status: Some("running".to_string()),
                    ..Default::default()
                },
            )
            .expect("reset feature status");
    };
    reset_for_retry(&ctx);
    ctx.features
        .sequence_checkpoint_record(
            &feature_id,
            "s-impl",
            &["stub-task-1".to_string(), "stub-task-2".to_string()],
            Some(anchor.as_str()),
            Some(&crate::domain::models::SequenceProduced {
                artifact_refs: vec![],
                satisfied_decls: vec!["implemented-files".to_string()],
            }),
            paths::now_ms(),
        )
        .expect("seed checkpoint");
    let before_resume = subtask_runs(&tmp, &feature_id).len();

    let ctx2 = build_core_context(
        CoreConfig {
            app_data_dir: tmp.clone(),
            execution_mode: ExecutionMode::LocalOnly,
        },
        Arc::new(NoopNotif),
        tokio::runtime::Handle::current(),
    );
    ctx2.executor
        .step_retry(impl_step.id.0.as_str(), None, None, None)
        .await
        .expect("retry the interrupted sequence step");
    assert_eq!(poll_terminal(&ctx2, &feature_id).await, "failed");

    // 1. The gate ran. Skipping it here reports `completed` with no
    //    deliverable, which is worse than the cost it was avoiding.
    let after = ctx2
        .features
        .steps_for_feature(&feature_id)
        .expect("steps")
        .into_iter()
        .find(|s| s.step_id.0 == "s-impl")
        .expect("sequence step exists");
    assert!(
        after
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("verification-report"),
        "a resumed step must still fail its never-produced deliverable; got {:?}",
        after.error_message
    );
    // The payload said `implemented-files` was produced, so the gate must
    // not fail on *that* one — it is judging real evidence, not an absence.
    assert!(
        !after
            .error_message
            .as_deref()
            .unwrap_or_default()
            .contains("implemented-files"),
        "the checkpoint recorded implemented-files as satisfied; got {:?}",
        after.error_message
    );

    // 2. The verdict discarded the checkpoint. Rewinding it would hand the
    //    next attempt the same zero-task resume forever.
    let checkpoint = ctx2
        .features
        .sequence_checkpoint_get(&feature_id, "s-impl")
        .expect("read checkpoint");
    assert!(
        checkpoint.is_empty(),
        "a verdict against a zero-task attempt must discard the checkpoint, not rewind to it; \
         found {:?}",
        checkpoint.landed_task_ids
    );

    // 3. Which means the step can actually move: the next attempt re-runs
    //    the list, so an agent finally sees the feedback.
    reset_for_retry(&ctx2);
    ctx2.executor
        .step_retry(impl_step.id.0.as_str(), None, None, None)
        .await
        .expect("retry again");
    assert_eq!(poll_terminal(&ctx2, &feature_id).await, "failed");
    let third: Vec<(String, String)> = subtask_runs(&tmp, &feature_id)
        .into_iter()
        .skip(before_resume)
        .collect();
    assert!(
        third.iter().any(|(id, _)| id.contains("stub-task-1")),
        "after the discard the tasks must run again, or the retry budget buys nothing; \
         ran: {third:?}"
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
