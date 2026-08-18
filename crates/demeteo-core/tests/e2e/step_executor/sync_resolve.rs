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
            "git -C /repos/demeteo_wt_sync_conflicted rev-parse --verify --quiet MERGE_HEAD",
            Ok("b1b2b3b\n"),
        ),
        (
            "git -C /repos/demeteo_wt_sync_conflicted status --porcelain",
            Ok("UU README.md\n"),
        ),
    ])
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
        .feature_resolve_sync_conflicts_impl(&feature_id, &["README.md".to_string()])
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
        exec.commands()
            .contains(&format!("git -C {WT} rev-parse --verify MERGE_HEAD")),
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
