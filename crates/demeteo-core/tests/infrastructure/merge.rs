// Tests extracted from `crates/demeteo-core/src/adapters/merge.rs` (mirrored-tests convention). `super` = that module.
//
// The `ExecutionPort` double errors on anything it was not scripted, so a sync
// that reached for git before it had decided whether it was allowed to start
// reddens rather than reading as an empty answer.

use super::*;

use crate::adapters::database::SqliteAdapter;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::sync_session::SyncSessionStatus;
use crate::ports::sync_session::SyncSessionPort;
use rusqlite::Connection;

const REPO: &str = "/repos/demeteo";
const WT: &str = "/repos/demeteo_wt_sync_feature-f-1";

fn fid() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

/// A feature with a project but deliberately **no** `repositories` row: the
/// only sync that can get as far as needing one is a sync this refusal let
/// through, so "which error comes back" is also the assertion that the guard
/// runs before anything is resolved or fetched.
fn executor(scripted: ScriptedExec) -> (SqliteMergeExecutor, Arc<SqliteAdapter>) {
    let db = Arc::new(SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap());
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO features (id, project_id, title, status, created_at)
             VALUES ('f-1', 'p-1', 'sync me', 'completed', 0)",
            [],
        )
        .unwrap();
    }
    let exec: Arc<dyn ExecutionPort> = Arc::new(scripted);
    let app_settings: Arc<dyn crate::ports::db::AppSettingsRepository> = db.clone();
    (
        SqliteMergeExecutor::new(
            db.clone(),
            db.clone(),
            GitOpsHelper::new(app_settings, exec.clone()),
            exec,
            std::path::PathBuf::from("/workspace"),
        ),
        db,
    )
}

fn session(status: SyncSessionStatus, pushed_at: Option<i64>) -> SyncSession {
    SyncSession {
        feature_id: "f-1".to_string(),
        machine_id: crate::domain::ids::LOCAL_MACHINE.to_string(),
        repo_dir: REPO.to_string(),
        feature_branch: "feature/f-1".to_string(),
        base_branch: "master".to_string(),
        status,
        worktree_path: Some(WT.to_string()),
        head_before: Some("aaaaaaa".to_string()),
        merge_commit_sha: Some("c0ffeec".to_string()),
        conflict_files: Vec::new(),
        raw_error: None,
        pushed_at,
        attempts: 0,
        created_at: 100,
        updated_at: 100,
    }
}

/// The reconcile probe for a tree holding a committed resolution.
fn resolved_probe() -> ScriptedExec {
    ScriptedExec::new(&[
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --git-dir",
            Ok(".git\n"),
        ),
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse --verify --quiet MERGE_HEAD",
            Ok(""),
        ),
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 status --porcelain",
            Ok(""),
        ),
        (
            "git -C /repos/demeteo_wt_sync_feature-f-1 rev-parse HEAD",
            Ok("c0ffeec\n"),
        ),
    ])
}

/// Pressing "Sync with main" over a resolution nobody has read used to retire it
/// silently. `open` is an upsert on one row per feature, so the new sync took
/// `head_before`, `merge_commit_sha` and `pushed_at` with it; the merge itself
/// then changed nothing — `origin/<base>` was already in the branch — so the
/// push was skipped and the row landed on a terminal `up_to_date`, from which
/// Publish and Discard are both refused. One click, and the merge is on its way
/// to the pull request with the only affordance that could have stopped it gone.
#[tokio::test]
async fn a_sync_may_not_start_over_a_resolution_nobody_has_read() {
    let (executor, db) = executor(resolved_probe());
    let sessions: &dyn SyncSessionPort = &*db;
    sessions
        .open(&session(SyncSessionStatus::Resolved, None))
        .unwrap();

    let failure = executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master")
        .await
        .expect_err("the held resolution is what stops it");
    match failure {
        UpstreamSyncFailure::Blocked { stage, raw_error } => {
            assert_eq!(stage, SyncBlockedStage::HeldResolution);
            assert!(
                raw_error.contains("Publish it or discard it"),
                "{raw_error}"
            );
        }
        other => panic!("{other:?}"),
    }

    let after = sessions.get(&fid()).unwrap().unwrap();
    assert_eq!(after.status, SyncSessionStatus::Resolved);
    assert_eq!(after.head_before.as_deref(), Some("aaaaaaa"));
    assert_eq!(after.merge_commit_sha.as_deref(), Some("c0ffeec"));
}

/// A resolution origin already has is nothing to protect, and the guard must
/// not stand between a feature and its next sync. This one gets as far as
/// wanting a repository row, which is exactly one step past the refusal.
#[tokio::test]
async fn a_published_resolution_does_not_stand_in_the_way_of_the_next_sync() {
    let (executor, db) = executor(resolved_probe());
    let sessions: &dyn SyncSessionPort = &*db;
    sessions
        .open(&session(SyncSessionStatus::Resolved, Some(200)))
        .unwrap();

    let failure = executor
        .sync_feature_with_upstream(&fid(), "feature/f-1", "master")
        .await
        .expect_err("this fixture configures no repository");
    match failure {
        UpstreamSyncFailure::Blocked { stage, .. } => {
            assert_eq!(stage, SyncBlockedStage::RepoContext)
        }
        other => panic!("{other:?}"),
    }
}
