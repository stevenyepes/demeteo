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
        sequence_checkpoint_record(
            &db,
            &f,
            "s-impl",
            &["a".into(), "b".into()],
            None,
            None,
            100
        )
        .unwrap(),
        2
    );
    // Second mid-list failure lands more tasks; duplicates fold away.
    assert_eq!(
        sequence_checkpoint_record(
            &db,
            &f,
            "s-impl",
            &["b".into(), "c".into()],
            None,
            None,
            200
        )
        .unwrap(),
        3
    );
    assert_eq!(
        sequence_checkpoint_get(&db, &f, "s-impl")
            .unwrap()
            .landed_task_ids,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}

/// The anchor names the *tip* of the landed prefix, so unlike the id list
/// it is replaced rather than merged — each task that lands moves it
/// forward.
#[test]
fn checkpoint_anchor_advances_with_the_prefix() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();
    sequence_checkpoint_record(&db, &f, "s-impl", &["b".into()], Some("sha-b"), None, 200).unwrap();

    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert_eq!(cp.landed_task_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(cp.anchor_sha.as_deref(), Some("sha-b"));
}

/// A caller that could not read a HEAD knows less than the row does, so
/// `None` must not blank an anchor an earlier task already recorded —
/// that would silently downgrade the row to "already merged, skip the
/// ids", which is a resume that drops work.
#[test]
fn recording_without_an_anchor_keeps_the_stored_one() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();
    sequence_checkpoint_record(&db, &f, "s-impl", &["b".into()], None, None, 200).unwrap();

    assert_eq!(
        sequence_checkpoint_get(&db, &f, "s-impl")
            .unwrap()
            .anchor_sha
            .as_deref(),
        Some("sha-a")
    );
}

/// A V32-era row carries no anchor, and must read back as one rather than
/// as an empty string the resume would try to `cat-file`.
#[test]
fn a_pre_v35_row_reads_back_without_an_anchor() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], None, None, 100).unwrap();

    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert!(!cp.is_empty());
    assert_eq!(cp.anchor_sha, None);
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
        sequence_checkpoint_record(
            &db,
            &fid("f-1"),
            "s-impl",
            &["stub-task-1".into()],
            Some("stub-sha"),
            None,
            100,
        )
        .unwrap();
    }
    // Life 2 (fresh adapter over the same file) hydrates it — ids *and*
    // the anchor, which is what tells life 2 where the work actually is.
    {
        let db = SqliteAdapter::new(Connection::open(&path).unwrap()).unwrap();
        let cp = sequence_checkpoint_get(&db, &fid("f-1"), "s-impl").unwrap();
        assert_eq!(cp.landed_task_ids, vec!["stub-task-1".to_string()]);
        assert_eq!(cp.anchor_sha.as_deref(), Some("stub-sha"));
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn checkpoints_are_scoped_per_feature_and_node() {
    let db = db();
    sequence_checkpoint_record(
        &db,
        &fid("f-1"),
        "s-a",
        &["t1".into()],
        Some("sha-1"),
        None,
        100,
    )
    .unwrap();
    sequence_checkpoint_record(
        &db,
        &fid("f-1"),
        "s-b",
        &["t2".into()],
        Some("sha-2"),
        None,
        100,
    )
    .unwrap();
    sequence_checkpoint_record(
        &db,
        &fid("f-2"),
        "s-a",
        &["t3".into()],
        Some("sha-3"),
        None,
        100,
    )
    .unwrap();
    for (feature, step, task, sha) in [
        ("f-1", "s-a", "t1", "sha-1"),
        ("f-1", "s-b", "t2", "sha-2"),
        ("f-2", "s-a", "t3", "sha-3"),
    ] {
        let cp = sequence_checkpoint_get(&db, &fid(feature), step).unwrap();
        assert_eq!(cp.landed_task_ids, vec![task.to_string()]);
        // The anchor is scoped with the ids: handing one node's commit to
        // another would reset a worktree onto an unrelated task list.
        assert_eq!(cp.anchor_sha.as_deref(), Some(sha));
    }
}

#[test]
fn clear_spends_the_checkpoint() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();
    sequence_checkpoint_clear(&db, &f, "s-impl").unwrap();
    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert!(cp.is_empty());
    assert_eq!(cp.anchor_sha, None);
    // Clearing a non-existent row is a no-op, not an error.
    sequence_checkpoint_clear(&db, &f, "s-impl").unwrap();
}

/// `set` is the only write that can make a checkpoint *smaller*, which is
/// what rewinding a discarded attempt needs. `record` would union the ids
/// straight back in and the rollback would achieve nothing.
#[test]
fn set_replaces_the_checkpoint_rather_than_unioning_it() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(
        &db,
        &f,
        "s-impl",
        &["a".into(), "b".into(), "c".into()],
        Some("sha-c"),
        None,
        100,
    )
    .unwrap();

    // The attempt that landed b and c is discarded; only a survives.
    sequence_checkpoint_set(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 200).unwrap();

    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert_eq!(cp.landed_task_ids, vec!["a".to_string()]);
    assert_eq!(cp.anchor_sha.as_deref(), Some("sha-a"));
}

/// Where `record`'s `None` means "I know less than the row does", `set`'s
/// means "there is no anchor" — the shape a rewind to an already-merged
/// prefix has to write, since an anchor left behind would send the next
/// attempt into a `reset --hard` it does not need.
#[test]
fn set_with_no_anchor_clears_the_stored_one() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();

    sequence_checkpoint_set(&db, &f, "s-impl", &["a".into()], None, None, 200).unwrap();

    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert_eq!(cp.landed_task_ids, vec!["a".to_string()]);
    assert_eq!(
        cp.anchor_sha, None,
        "a merged prefix must rewind to the anchor-less V32 shape"
    );
}

/// A rewind can be the first write this (feature, node) ever sees — the
/// attempt read a row, the row was cleared elsewhere, the rollback still
/// has to land somewhere. Insert, don't fail.
#[test]
fn set_inserts_when_no_row_exists() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_set(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();
    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert_eq!(cp.landed_task_ids, vec!["a".to_string()]);
    assert_eq!(cp.anchor_sha.as_deref(), Some("sha-a"));
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

/// The produced payload unions across the tasks that land, so the row
/// always describes the whole landed prefix rather than the last task.
#[test]
fn produced_unions_across_landed_tasks() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(
        &db,
        &f,
        "s-impl",
        &["a".into()],
        Some("sha-a"),
        Some(&CheckpointProduced {
            artifact_refs: vec!["/store/diff".into()],
            satisfied_decls: vec!["notes".into()],
        }),
        100,
    )
    .unwrap();
    sequence_checkpoint_record(
        &db,
        &f,
        "s-impl",
        &["b".into()],
        Some("sha-b"),
        Some(&CheckpointProduced {
            // The diff reference repeats — a stable path the store returns
            // again — and must fold away rather than accumulate.
            artifact_refs: vec!["/store/diff".into(), "/store/report.md".into()],
            satisfied_decls: vec!["report".into()],
        }),
        200,
    )
    .unwrap();

    let produced = sequence_checkpoint_get(&db, &f, "s-impl")
        .unwrap()
        .produced
        .expect("the row carries a payload");
    assert_eq!(
        produced.artifact_refs,
        vec!["/store/diff".to_string(), "/store/report.md".to_string()]
    );
    assert_eq!(
        produced.satisfied_decls,
        vec!["notes".to_string(), "report".to_string()]
    );
}

/// A pre-V36 row that already claims tasks cannot be completed from here:
/// unioning one task's output into it yields a set that *looks* whole while
/// omitting the earlier tasks' artifacts, and the declared-deliverable
/// check would then fail a step for a deliverable that exists. Such a row
/// stays unknown, and the resume keeps its pre-V36 behaviour.
#[test]
fn a_pre_v36_row_stays_unknown_rather_than_becoming_partial() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();

    sequence_checkpoint_record(
        &db,
        &f,
        "s-impl",
        &["b".into()],
        Some("sha-b"),
        Some(&CheckpointProduced {
            artifact_refs: vec!["/store/report.md".into()],
            satisfied_decls: vec!["report".into()],
        }),
        200,
    )
    .unwrap();

    assert_eq!(
        sequence_checkpoint_get(&db, &f, "s-impl").unwrap().produced,
        None,
        "a partial payload must not be presented as a complete one"
    );
}

/// A row whose payload will not parse reads back as unknown rather than
/// failing the whole checkpoint read — the resume then re-runs at worst,
/// where an error would abandon a row that could still shorten the list.
#[test]
fn an_unparseable_payload_degrades_to_unknown() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(&db, &f, "s-impl", &["a".into()], Some("sha-a"), None, 100).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE sequence_checkpoints SET produced_json = 'not json'
             WHERE feature_id = 'f-1' AND step_id = 's-impl'",
            [],
        )
        .unwrap();
    }

    let cp = sequence_checkpoint_get(&db, &f, "s-impl").unwrap();
    assert_eq!(cp.landed_task_ids, vec!["a".to_string()]);
    assert_eq!(cp.produced, None);
}

/// A rewind shrinks the payload with the ids. Leaving this attempt's
/// artifact references behind would advertise output belonging to commits
/// the rollback discarded.
#[test]
fn set_replaces_the_produced_payload_too() {
    let db = db();
    let f = fid("f-1");
    sequence_checkpoint_record(
        &db,
        &f,
        "s-impl",
        &["a".into(), "b".into()],
        Some("sha-b"),
        Some(&CheckpointProduced {
            artifact_refs: vec!["/store/a".into(), "/store/b".into()],
            satisfied_decls: vec!["notes".into(), "report".into()],
        }),
        100,
    )
    .unwrap();

    let kept = CheckpointProduced {
        artifact_refs: vec!["/store/a".into()],
        satisfied_decls: vec!["notes".into()],
    };
    sequence_checkpoint_set(
        &db,
        &f,
        "s-impl",
        &["a".into()],
        Some("sha-a"),
        Some(&kept),
        200,
    )
    .unwrap();

    assert_eq!(
        sequence_checkpoint_get(&db, &f, "s-impl").unwrap().produced,
        Some(kept)
    );
}
