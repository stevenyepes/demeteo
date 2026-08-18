// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/sync_session.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::models::ConflictFile;
use rusqlite::Connection;

/// Seeded with the parent rows because foreign keys are enforced here: the
/// session cascades off `features`, and an orphan row is not a state the
/// schema allows.
fn db() -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    seed_feature(&db);
    db
}

fn seed_feature(db: &SqliteAdapter) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO features (id, project_id, title, created_at)
         VALUES ('f-1', 'p-1', 'sync me', 0)",
        [],
    )
    .unwrap();
}

fn fid() -> FeatureId {
    FeatureId::from("f-1".to_string())
}

fn conflicted() -> SyncSession {
    SyncSession {
        feature_id: "f-1".to_string(),
        machine_id: "local".to_string(),
        repo_dir: "/repos/demeteo".to_string(),
        feature_branch: "feature/f-1".to_string(),
        base_branch: "master".to_string(),
        status: SyncSessionStatus::Conflicted,
        worktree_path: Some("/repos/demeteo_wt_sync_feature-f-1".to_string()),
        head_before: Some("aaaaaaa".to_string()),
        merge_commit_sha: None,
        conflict_files: vec![ConflictFile {
            path: "src/lib.rs".to_string(),
            kind: "both modified".to_string(),
        }],
        raw_error: Some("CONFLICT (content): Merge conflict in src/lib.rs".to_string()),
        attempts: 0,
        created_at: 100,
        updated_at: 100,
    }
}

#[test]
fn a_conflict_survives_the_write_and_reads_back_whole() {
    let db = db();
    db.open(&conflicted()).unwrap();

    let read = SyncSessionPort::get(&db, &fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Conflicted);
    assert_eq!(
        read.worktree_path.as_deref(),
        Some("/repos/demeteo_wt_sync_feature-f-1")
    );
    assert_eq!(read.head_before.as_deref(), Some("aaaaaaa"));
    assert_eq!(read.repo_dir, "/repos/demeteo");
    assert_eq!(read.conflict_files.len(), 1);
    assert_eq!(read.conflict_files[0].path, "src/lib.rs");
    assert!(read.raw_error.unwrap().contains("src/lib.rs"));
}

/// One live sync per feature is what the primary key is for: a second attempt
/// replaces the first rather than accumulating a second answer to "is this
/// feature mid-sync".
#[test]
fn reopening_replaces_the_previous_session_rather_than_adding_one() {
    let db = db();
    db.open(&conflicted()).unwrap();
    let mut second = conflicted();
    second.status = SyncSessionStatus::Syncing;
    second.worktree_path = None;
    second.conflict_files = Vec::new();
    db.open(&second).unwrap();

    let read = SyncSessionPort::get(&db, &fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Syncing);
    assert_eq!(read.worktree_path, None);
    assert!(read.conflict_files.is_empty());
}

/// The patch's three states, on the one column where all three matter: a
/// transition that has nothing to say about the worktree must not blank it,
/// and an abort that clears it must actually clear it.
#[test]
fn an_untouched_field_keeps_its_value_and_a_cleared_one_goes_null() {
    let db = db();
    db.open(&conflicted()).unwrap();

    db.update(
        &fid(),
        &SyncSessionPatch {
            status: Some(SyncSessionStatus::Resolving),
            bump_attempts: true,
            ..Default::default()
        },
        200,
    )
    .unwrap();
    let read = SyncSessionPort::get(&db, &fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Resolving);
    assert_eq!(
        read.worktree_path.as_deref(),
        Some("/repos/demeteo_wt_sync_feature-f-1")
    );
    assert_eq!(read.head_before.as_deref(), Some("aaaaaaa"));
    assert_eq!(read.attempts, 1);
    assert_eq!(read.updated_at, 200);

    db.update(
        &fid(),
        &SyncSessionPatch {
            status: Some(SyncSessionStatus::Aborted),
            worktree_path: Some(None),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    let read = SyncSessionPort::get(&db, &fid()).unwrap().unwrap();
    assert_eq!(read.worktree_path, None);
    assert_eq!(read.head_before.as_deref(), Some("aaaaaaa"));
    assert_eq!(read.attempts, 1, "an unbumped patch must not re-count");
}

/// A status this build cannot name is one it cannot act on, and the worktree
/// that row points at is not ours to keep alive — so it reads as abandoned
/// rather than panicking or being believed.
#[test]
fn a_status_written_by_a_newer_build_degrades_to_abandoned() {
    let db = db();
    db.open(&conflicted()).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE sync_sessions SET status = 'rebasing' WHERE feature_id = 'f-1'",
            [],
        )
        .unwrap();
    }
    let read = SyncSessionPort::get(&db, &fid()).unwrap().unwrap();
    assert_eq!(read.status, SyncSessionStatus::Aborted);
}

#[test]
fn an_absent_session_is_none_rather_than_an_error() {
    let db = db();
    assert!(SyncSessionPort::get(&db, &fid()).unwrap().is_none());
    db.open(&conflicted()).unwrap();
    db.close(&fid()).unwrap();
    assert!(SyncSessionPort::get(&db, &fid()).unwrap().is_none());
}

/// A sync belongs to its feature, so deleting the feature takes it with it —
/// otherwise the orphan keeps naming a worktree that a re-created feature of
/// the same id would then inherit.
#[test]
fn deleting_the_feature_takes_its_session_with_it() {
    let db = db();
    db.open(&conflicted()).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("DELETE FROM features WHERE id = 'f-1'", [])
            .unwrap();
    }
    assert!(SyncSessionPort::get(&db, &fid()).unwrap().is_none());
}

/// The other half of the same constraint: a session for a feature that does
/// not exist is not a session.
#[test]
fn a_session_for_a_feature_that_does_not_exist_is_refused() {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    assert!(db.open(&conflicted()).is_err());
}
