// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/sequence_state.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn fid(s: &str) -> FeatureId {
    FeatureId::from(s.to_string())
}

#[test]
fn checkpoint_records_union_in_landed_order() {
    let db = db();
    let f = fid("f-1");
    assert_eq!(
        sequence_checkpoint_record(&db, &f, "s-impl", &["a".into(), "b".into()], 100).unwrap(),
        2
    );
    // Second mid-list failure lands more tasks; duplicates fold away.
    assert_eq!(
        sequence_checkpoint_record(&db, &f, "s-impl", &["b".into(), "c".into()], 200).unwrap(),
        3
    );
    assert_eq!(
        sequence_checkpoint_get(&db, &f, "s-impl").unwrap(),
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

/// The whole point of V32: a second driver life (fresh in-memory state,
/// same DB) sees the checkpoint the first life recorded — a restart
/// resumes from the exact task, not the step head.
#[test]
fn checkpoint_survives_across_driver_lives() {
    let tmp = std::env::temp_dir().join(format!(
        "demeteo-seq-state-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let path = tmp.join("db.sqlite");

    // Life 1 writes the checkpoint, then the process "dies".
    {
        let db = SqliteAdapter::new(Connection::open(&path).unwrap()).unwrap();
        sequence_checkpoint_record(&db, &fid("f-1"), "s-impl", &["stub-task-1".into()], 100)
            .unwrap();
    }
    // Life 2 (fresh adapter over the same file) hydrates it.
    {
        let db = SqliteAdapter::new(Connection::open(&path).unwrap()).unwrap();
        assert_eq!(
            sequence_checkpoint_get(&db, &fid("f-1"), "s-impl").unwrap(),
            vec!["stub-task-1".to_string()]
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn checkpoints_are_scoped_per_feature_and_node() {
    let db = db();
    sequence_checkpoint_record(&db, &fid("f-1"), "s-a", &["t1".into()], 100).unwrap();
    sequence_checkpoint_record(&db, &fid("f-1"), "s-b", &["t2".into()], 100).unwrap();
    sequence_checkpoint_record(&db, &fid("f-2"), "s-a", &["t3".into()], 100).unwrap();
    assert_eq!(
        sequence_checkpoint_get(&db, &fid("f-1"), "s-a").unwrap(),
        vec!["t1".to_string()]
    );
    assert_eq!(
        sequence_checkpoint_get(&db, &fid("f-1"), "s-b").unwrap(),
        vec!["t2".to_string()]
    );
    assert_eq!(
        sequence_checkpoint_get(&db, &fid("f-2"), "s-a").unwrap(),
        vec!["t3".to_string()]
    );
}

#[test]
fn clear_spends_the_checkpoint() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], 100).unwrap();
    sequence_checkpoint_clear(&db, &f, "s-impl").unwrap();
    assert!(sequence_checkpoint_get(&db, &f, "s-impl")
        .unwrap()
        .is_empty());
    // Clearing a non-existent row is a no-op, not an error.
    sequence_checkpoint_clear(&db, &f, "s-impl").unwrap();
}

#[test]
fn plan_cache_roundtrips_and_upserts() {
    let db = db();
    let f = fid("f-1");
    assert_eq!(plan_cache_get(&db, &f, "s-impl").unwrap(), None);

    plan_cache_put(&db, &f, "s-impl", r#"{"tasks":[]}"#, Some(1), 100).unwrap();
    assert_eq!(
        plan_cache_get(&db, &f, "s-impl").unwrap().as_deref(),
        Some(r#"{"tasks":[]}"#)
    );

    // A later attempt's full re-plan replaces the row.
    plan_cache_put(&db, &f, "s-impl", r#"{"tasks":[{"id":"t"}]}"#, Some(2), 200).unwrap();
    assert_eq!(
        plan_cache_get(&db, &f, "s-impl").unwrap().as_deref(),
        Some(r#"{"tasks":[{"id":"t"}]}"#)
    );
    let conn = db.conn.lock().unwrap();
    let (attempt_no, updated_at): (Option<u32>, i64) = conn
        .query_row(
            "SELECT attempt_no, updated_at FROM sequence_plan_cache
             WHERE feature_id = 'f-1' AND step_id = 's-impl'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(attempt_no, Some(2));
    assert_eq!(updated_at, 200);
}
