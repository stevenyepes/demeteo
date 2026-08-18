//! Where the "Resolve with agent" button gets its facts, and when it refuses.
//!
//! The worktree came out of a string search over the newest `conflict` row of
//! the `feature_syncs` audit, which answered `None` the moment a later attempt
//! failed before merging — and the fallback for that was checking the feature
//! branch out in the user's own clone and letting an agent loose in it. V43's
//! session row is the answer now, and these tests are what stop the old one
//! coming back: they assert the directory git was actually asked about.
//!
//! Nothing past the resolver's own preflight is scripted, so the turn stops
//! there. Which directory it stopped in is the subject.

use std::sync::Arc;

use super::harness::{build_test_executor_in, scratch_dir, FakeNotif};
use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{FeatureId, ProjectId, LOCAL_MACHINE};
use crate::domain::models::StepExecution;
use crate::domain::step_seed::{manual_sync_step_execution, MANUAL_SYNC_STEP_ID};
use crate::domain::sync_session::SyncSessionStatus;
use crate::paths;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::step_executor::SyncOutcomeView;
use crate::ports::sync_session::{SyncSession, SyncSessionPort};

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo_wt_sync_conflicted";

/// What `application::sync_session` asks of the worktree before anyone is
/// allowed to believe the row. An open merge over a dirty tree keeps a
/// `conflicted` session exactly as stored.
fn live_conflict_probes() -> ScriptedExec {
    ScriptedExec::new(&[
        (
            "git -C /repos/demeteo_wt_sync_conflicted rev-parse --git-dir",
            Ok(".git\n"),
        ),
        (
            "git -C /repos/demeteo_wt_sync_conflicted status --porcelain",
            Ok("UU README.md\n"),
        ),
    ])
    // The reconcile probe and the resolver's preflight ask the *same* question
    // of the same directory, so the queue is what lets one answer and the other
    // refuse: the session is read as a live conflict, and then the turn stops at
    // its own preflight rather than spawning an agent nothing here scripts.
    .with_queue(
        "git -C /repos/demeteo_wt_sync_conflicted rev-parse --verify --quiet MERGE_HEAD",
        &[Ok("b1b2b3b\n"), Err("fatal: Needed a single revision")],
    )
}

/// One project and one feature — deliberately no `repositories` row: every path
/// the resolver needs now comes off the session, and a test that seeded one
/// would not notice the day it started resolving the repo dir again.
fn seed(db: &Arc<SqliteAdapter>, label: &str, feature_status: &str) -> String {
    let project_id = format!("p-{label}");
    let projects: &dyn ProjectRepository = &**db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from(project_id.clone()),
            name: label.to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: paths::now_ms(),
        })
        .expect("add project");

    let feature_id = format!("f-{label}");
    let features: &dyn FeatureRepository = &**db;
    features
        .add(crate::domain::models::Feature {
            id: FeatureId::from(feature_id.clone()),
            project_id: ProjectId::from(project_id),
            workflow_id: None,
            workflow_version_id: None,
            title: "Resolve me".to_string(),
            description: String::new(),
            status: feature_status.to_string(),
            total_cost: 0.0,
            tokens: 0,
            duration: "0s".to_string(),
            created_at: paths::now_ms(),
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts: None,
            loop_iterations: None,
            max_budget_usd: None,
            step_overrides: Vec::new(),
            attachments: Vec::new(),
            harness_baseline: None,
            origin: FeatureOrigin::DefaultBranch,
            diff_base_branch: Some("master".to_string()),
            resolved_branch: Some(format!("demeteo/features/{feature_id}")),
        })
        .expect("add feature");

    let sessions: &dyn SyncSessionPort = &**db;
    sessions
        .open(&SyncSession {
            feature_id: feature_id.clone(),
            machine_id: LOCAL_MACHINE.to_string(),
            repo_dir: REPO.to_string(),
            feature_branch: format!("demeteo/features/{feature_id}"),
            base_branch: "master".to_string(),
            status: SyncSessionStatus::Conflicted,
            worktree_path: Some(WT.to_string()),
            head_before: Some("aaaaaaa".to_string()),
            merge_commit_sha: None,
            conflict_files: Vec::new(),
            raw_error: Some("CONFLICT (content): Merge conflict in README.md".to_string()),
            attempts: 0,
            created_at: paths::now_ms(),
            updated_at: paths::now_ms(),
        })
        .expect("open the session");

    feature_id
}

/// Press the button on a feature whose sync is conflicted.
async fn resolve(
    label: &str,
    feature_status: &str,
) -> (
    Result<SyncOutcomeView, String>,
    Arc<ScriptedExec>,
    Arc<SqliteAdapter>,
    std::path::PathBuf,
) {
    let temp_dir = scratch_dir(label);
    let exec = Arc::new(live_conflict_probes());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed(&db, label, feature_status);

    let view = executor
        .feature_resolve_sync_conflicts_impl(
            &feature_id,
            &["README.md".to_string()],
            &Default::default(),
        )
        .await;
    (view, exec, db, temp_dir)
}

fn stored_status(db: &Arc<SqliteAdapter>, feature_id: &str) -> SyncSessionStatus {
    let sessions: &dyn SyncSessionPort = &**db;
    sessions
        .get(&FeatureId::from(feature_id.to_string()))
        .expect("read the session")
        .expect("the session is still there")
        .status
}

#[tokio::test]
async fn the_resolver_works_in_the_worktree_the_session_names() {
    let (view, exec, _db, temp_dir) = resolve("resolve_worktree", "completed").await;

    assert!(
        matches!(view, Ok(SyncOutcomeView::ResolutionFailed { .. })),
        "the unscripted preflight is a resolution that did not land: {view:?}"
    );
    assert!(
        exec.commands().contains(&format!(
            "git -C {WT} rev-parse --verify --quiet MERGE_HEAD"
        )),
        "the resolver has to look for the merge where the session put it: {:?}",
        exec.commands()
    );
    assert!(
        !exec
            .commands()
            .iter()
            .any(|c| c.contains(&format!("git -C {REPO} "))),
        "nothing may run in the user's own clone: {:?}",
        exec.commands()
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The row said `conflicted` before the button and has to say something else
/// after it. It said `conflicted` forever: only the workflow step recorded its
/// verdict, so a resolution the user asked for left the feature looking, to the
/// banner and to `sync_abort` alike, like a conflict still waiting for them.
#[tokio::test]
async fn a_manual_resolution_files_its_verdict_on_the_session() {
    let (_, _exec, db, temp_dir) = resolve("resolve_verdict", "completed").await;

    assert_eq!(
        stored_status(&db, "f-resolve_verdict"),
        SyncSessionStatus::ResolutionFailed
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The UI stopped offering this while a run is live, which leaves the IPC as
/// the only thing between a second agent and a worktree the first one is
/// writing in.
#[tokio::test]
async fn a_sync_the_run_owns_is_not_the_buttons_to_resolve() {
    let (view, exec, db, temp_dir) = resolve("resolve_owned", "running").await;

    match view {
        Err(reason) => assert!(reason.contains("still going"), "{reason}"),
        other => panic!("a live run's sync was resolvable from the button: {other:?}"),
    }
    assert_eq!(
        stored_status(&db, "f-resolve_owned"),
        SyncSessionStatus::Conflicted,
        "a refusal may not move the session"
    );
    assert!(
        !exec
            .commands()
            .iter()
            .any(|c| c.contains("--untracked-files=no")),
        "the refusal has to come before the resolver's own preflight: {:?}",
        exec.commands()
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

fn steps_of(db: &Arc<SqliteAdapter>, feature_id: &str) -> Vec<StepExecution> {
    let features: &dyn FeatureRepository = &**db;
    features
        .steps_for_feature(&FeatureId::from(feature_id.to_string()))
        .expect("read the rows")
}

/// The turn used to stream against `se-sync-<millis>`, a step-execution id no
/// row carried. The inspector only ever subscribes to ids `step_list_for_run`
/// handed it, so every token the resolver emitted went to a buffer nothing
/// renders and the button was a spinner with no output, no cost and no cancel.
#[tokio::test]
async fn the_manual_turn_reports_through_a_row_the_run_can_see() {
    let (_, _exec, db, temp_dir) = resolve("resolve_row", "completed").await;

    let rows = steps_of(&db, "f-resolve_row");
    let row = rows
        .iter()
        .find(|r| r.step_id.0 == MANUAL_SYNC_STEP_ID)
        .unwrap_or_else(|| panic!("no row for the manual sync: {rows:?}"));
    assert_eq!(row.id.0, "se-f-resolve_row-s-sync-manual");
    assert_eq!(row.step_kind, "sync");
    assert_eq!(
        row.status, "failed",
        "the preflight is unscripted, so it fails"
    );
    assert_eq!(row.step_index, u32::MAX);
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// `step_create` is a bare `INSERT`, so the second attempt has to find the row
/// the first one left rather than mint a colliding one.
#[tokio::test]
async fn a_second_manual_sync_reuses_the_first_ones_row() {
    let temp_dir = scratch_dir("resolve_twice");
    let exec = Arc::new(live_conflict_probes());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed(&db, "resolve_twice", "completed");

    for _ in 0..2 {
        executor
            .feature_resolve_sync_conflicts_impl(
                &feature_id,
                &["README.md".to_string()],
                &Default::default(),
            )
            .await
            .expect("the button answers a view, never an insert error");
    }

    let manual = steps_of(&db, &feature_id)
        .into_iter()
        .filter(|r| r.step_id.0 == MANUAL_SYNC_STEP_ID)
        .count();
    assert_eq!(manual, 1);
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The row is reused by every out-of-band sync a feature runs, and the header's
/// spend is a sum over rows — so opening it at zero makes the previous
/// attempt's dollars vanish from the feature's total. The run loop carries a
/// re-dispatched node's spend forward for the same reason.
#[tokio::test]
async fn a_repeat_resolution_does_not_erase_what_the_first_one_spent() {
    let temp_dir = scratch_dir("resolve_spend");
    let exec = Arc::new(live_conflict_probes());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed(&db, "resolve_spend", "completed");
    let features: &dyn FeatureRepository = &*db;
    let mut spent = manual_sync_step_execution(&FeatureId::from(feature_id.clone()), 0);
    spent.status = "failed".to_string();
    spent.cost_usd = Some(4.5);
    spent.tokens = Some(3000);
    features
        .step_create(spent)
        .expect("the first attempt's row");

    executor
        .feature_resolve_sync_conflicts_impl(
            &feature_id,
            &["README.md".to_string()],
            &Default::default(),
        )
        .await
        .expect("the button answers a view");

    let row = steps_of(&db, &feature_id)
        .into_iter()
        .find(|r| r.step_id.0 == MANUAL_SYNC_STEP_ID)
        .expect("the row is still there");
    assert_eq!(row.cost_usd, Some(4.5));
    assert_eq!(row.tokens, Some(3000));
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// A manual sync runs on a feature that already finished, and the restart
/// reconciliation reads only features in `running`/`gated` and the `pending`
/// rows of ones that were cancelled or failed. Its row would otherwise read
/// `running` forever, with nothing left in the process that could move it.
#[tokio::test]
async fn a_crashed_manual_sync_is_reconciled_on_the_next_start() {
    let temp_dir = scratch_dir("resolve_crash");
    let exec = Arc::new(live_conflict_probes());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed(&db, "resolve_crash", "completed");
    let features: &dyn FeatureRepository = &*db;
    let mut abandoned = manual_sync_step_execution(&FeatureId::from(feature_id.clone()), 0);
    abandoned.status = "running".to_string();
    features.step_create(abandoned).expect("leave a row behind");

    executor.startup_watchdog();

    let row = steps_of(&db, &feature_id)
        .into_iter()
        .find(|r| r.step_id.0 == MANUAL_SYNC_STEP_ID)
        .expect("the row is still there");
    assert_eq!(row.status, "interrupted");
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The inspector mounts on this row like any other and offers Retry and Replay,
/// and neither has anything to walk: the id is in no graph, so both fall back to
/// `step_index` — `u32::MAX` — which rewinds only this row and promotes every
/// real node to an ancestor to be restored, then sets the feature `running` and
/// arms a driver. On a finished run that is the whole outcome rewritten.
#[tokio::test]
async fn the_manual_sync_row_is_not_a_node_to_retry() {
    use crate::ports::step_executor::StepExecutor;

    let temp_dir = scratch_dir("resolve_retry");
    let exec = Arc::new(live_conflict_probes());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed(&db, "resolve_retry", "completed");
    let features: &dyn FeatureRepository = &*db;
    let mut failed = manual_sync_step_execution(&FeatureId::from(feature_id.clone()), 0);
    failed.status = "failed".to_string();
    let row_id = failed.id.0.clone();
    features
        .step_create(failed)
        .expect("the row the button left");

    let refusal = executor
        .step_retry(&row_id, None, None, None)
        .await
        .expect_err("a row no workflow contains was retried");
    assert!(
        format!("{refusal}").contains("out-of-band sync"),
        "{refusal:?}"
    );
    assert_eq!(
        features
            .get(&FeatureId::from(feature_id))
            .unwrap()
            .unwrap()
            .status,
        "completed",
        "the refusal may not re-arm a run that already finished"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// Two resolutions of one feature at once are the thing `user_may_intervene`
/// exists to prevent, and the session cannot refuse the second on its own:
/// `reconcile` rewrites a `resolving` row back to `conflicted` whenever the
/// merge is still open, which is sound for a row whose writer is gone and false
/// while this very process holds it. The in-flight entry is what serialises
/// them — and displacing it would also have swallowed the first turn's Stop.
#[tokio::test]
async fn a_second_resolution_is_refused_while_the_first_is_in_flight() {
    let temp_dir = scratch_dir("resolve_race");
    let exec = Arc::new(live_conflict_probes());
    let (executor, db) =
        build_test_executor_in(temp_dir.clone(), Arc::new(FakeNotif), exec.clone()).await;
    let feature_id = seed(&db, "resolve_race", "completed");

    let (tx, _rx) = tokio::sync::watch::channel(false);
    executor.claim_sync_cancel_for_test(&feature_id, tx);

    let refusal = executor
        .feature_resolve_sync_conflicts_impl(
            &feature_id,
            &["README.md".to_string()],
            &Default::default(),
        )
        .await
        .expect_err("a second agent was let into the same worktree");
    assert!(refusal.contains("already running"), "{refusal}");
    assert_eq!(
        stored_status(&db, &feature_id),
        SyncSessionStatus::Conflicted,
        "a refusal may not move the session"
    );
    let _ = std::fs::remove_dir_all(temp_dir);
}

/// The reconcile's live-conflict answers, plus the resolver's own preflight
/// read, so the turn reaches the spawn instead of stopping before it.
fn merge_open_for_the_resolver() -> ScriptedExec {
    ScriptedExec::new(&[
        (
            "git -C /repos/demeteo_wt_sync_conflicted rev-parse --git-dir",
            Ok(".git\n"),
        ),
        (
            "git -C /repos/demeteo_wt_sync_conflicted status --porcelain",
            Ok("UU README.md\n"),
        ),
        (
            "git -C /repos/demeteo_wt_sync_conflicted rev-parse --verify --quiet MERGE_HEAD",
            Ok("b1b2b3b\n"),
        ),
        // `preflight` returns on a non-empty unmerged list, so this is the last
        // thing asked before the spawn.
        (
            "git -C /repos/demeteo_wt_sync_conflicted status --porcelain --untracked-files=no",
            Ok("UU README.md\n"),
        ),
    ])
}

/// Records the identity one spawn was asked for, then refuses. Refusing is the
/// point: nothing past the spawn is scripted, so the turn stops at the one
/// question these tests are about.
struct RecordingRuntime {
    kind: &'static str,
    seen: std::sync::Mutex<Vec<Spawned>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Spawned {
    binary: String,
    model: Option<String>,
    effort: Option<crate::domain::models::EffortLevel>,
}

#[async_trait::async_trait]
impl crate::ports::agent_runtime::AgentRuntime for RecordingRuntime {
    fn kind(&self) -> &'static str {
        self.kind
    }
    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: "Recording",
            lists_models: false,
            model_listing: None,
            default_model: None,
            effort_levels: &[],
            personalization: crate::ports::agent_runtime::PersonalizationSupport::Native,
            windows_agent_shell: crate::domain::models::WindowsAgentShell::Unknown,
        }
    }
    async fn availability(
        &self,
        _exec: &dyn crate::ports::execution::ExecutionPort,
        _machine_id: &str,
    ) -> crate::domain::models::Availability {
        crate::domain::models::Availability::Installed
    }
    fn install_command(&self) -> &'static str {
        "echo recording"
    }
    fn start(
        &self,
        ctx: crate::ports::agent_runtime::AgentContext,
    ) -> crate::ports::agent_runtime::AgentStartFuture<'_> {
        self.seen.lock().unwrap().push(Spawned {
            binary: ctx.binary.clone(),
            model: ctx.model.clone(),
            effort: ctx.effort,
        });
        Box::pin(async move {
            Err(crate::ports::agent_runtime::AgentStartError::SpawnFailed(
                "recorded".to_string(),
            ))
        })
    }
}

/// Press the button on a conflicted feature whose registry can answer, and
/// report what the spawn was asked for.
async fn spawned_by(
    label: &str,
    kind: &'static str,
    prepare: impl FnOnce(&Arc<SqliteAdapter>, &crate::domain::models::Feature),
    asked: crate::domain::sync_resolver::SyncResolverChoice,
) -> Vec<Spawned> {
    let temp_dir = scratch_dir(label);
    let exec = Arc::new(merge_open_for_the_resolver());
    let runtime = Arc::new(RecordingRuntime {
        kind,
        seen: std::sync::Mutex::new(Vec::new()),
    });
    let (executor, db) = super::harness::build_test_executor_with_agents(
        temp_dir.clone(),
        Arc::new(FakeNotif),
        exec.clone(),
        vec![runtime.clone()],
    )
    .await;
    let feature_id = seed(&db, label, "completed");
    let features: &dyn FeatureRepository = &*db;
    let feature = features
        .get(&FeatureId::from(feature_id.clone()))
        .unwrap()
        .unwrap();
    prepare(&db, &feature);

    let _ = executor
        .feature_resolve_sync_conflicts_impl(&feature_id, &["README.md".to_string()], &asked)
        .await;

    let seen = runtime.seen.lock().unwrap().clone();
    let _ = std::fs::remove_dir_all(temp_dir);
    seen
}

fn resolver_settings(
    db: &Arc<SqliteAdapter>,
    feature: &crate::domain::models::Feature,
    choice: crate::domain::sync_resolver::SyncResolverChoice,
) {
    let projects: &dyn ProjectRepository = &**db;
    let mut settings = crate::adapters::step_executor::setup::fetch_default_settings();
    settings.project_id = feature.project_id.clone();
    settings.sync_resolver_agent_kind = choice.agent_kind;
    settings.sync_resolver_model = choice.model;
    settings.sync_resolver_effort = choice.effort;
    projects.save_settings(settings).unwrap();
}

/// The project names a conflict resolver; the run was launched with a different
/// harness. The turn has to spawn the project's, at the project's model and
/// effort — which is the whole claim of the resolver being a role and not a
/// step.
///
/// Both sync paths used to terminate at a hard-coded `"opencode"` without ever
/// reading `ProjectSettings`, so this also reddens if the chain is ever
/// short-circuited back to the feature row.
#[tokio::test]
async fn the_projects_conflict_resolver_outranks_the_runs_launch_pin() {
    use crate::domain::models::EffortLevel;
    use crate::domain::sync_resolver::SyncResolverChoice;

    let seen = spawned_by(
        "resolve_pick",
        "codex",
        |db, feature| {
            let features: &dyn FeatureRepository = &**db;
            features
                .update(
                    &feature.id,
                    &crate::ports::db::FeaturePatch {
                        agent_kind: Some(Some("opencode".to_string())),
                        model: Some(Some("sonnet".to_string())),
                        effort: Some(Some(EffortLevel::Max)),
                        ..Default::default()
                    },
                )
                .unwrap();
            resolver_settings(
                db,
                feature,
                SyncResolverChoice {
                    agent_kind: Some("codex".to_string()),
                    model: Some("gpt-5-codex".to_string()),
                    effort: Some(EffortLevel::Low),
                },
            );
        },
        SyncResolverChoice::default(),
    )
    .await;

    assert_eq!(
        seen,
        vec![Spawned {
            binary: "codex".to_string(),
            model: Some("gpt-5-codex".to_string()),
            effort: Some(EffortLevel::Low),
        }],
        "the project's resolver default never reached the spawn"
    );
}

/// What the user picked on the banner outranks everything stored — the tier the
/// picker exists to fill.
#[tokio::test]
async fn the_attempts_own_choice_outranks_the_projects_resolver_default() {
    use crate::domain::models::EffortLevel;
    use crate::domain::sync_resolver::SyncResolverChoice;

    let seen = spawned_by(
        "resolve_asked",
        "pi",
        |db, feature| {
            resolver_settings(
                db,
                feature,
                SyncResolverChoice {
                    agent_kind: Some("codex".to_string()),
                    ..Default::default()
                },
            );
        },
        SyncResolverChoice {
            agent_kind: Some("pi".to_string()),
            model: Some("pi-large".to_string()),
            effort: Some(EffortLevel::XHigh),
        },
    )
    .await;

    assert_eq!(
        seen,
        vec![Spawned {
            binary: "pi".to_string(),
            model: Some("pi-large".to_string()),
            effort: Some(EffortLevel::XHigh),
        }],
        "the picker's choice never reached the spawn"
    );
}
