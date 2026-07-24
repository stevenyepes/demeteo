// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/step_attempts.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use rusqlite::Connection;

fn db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn sid(s: &str) -> StepExecutionId {
    StepExecutionId::from(s.to_string())
}

#[test]
fn attempt_numbers_are_dense_and_one_based() {
    let db = db();
    let id = sid("se-1");
    assert_eq!(attempt_open(&db, &id, 100, None).unwrap(), 1);
    attempt_close(
        &db,
        &id,
        1,
        "failed",
        0.1,
        10,
        500,
        Some("verdict"),
        None,
        Some("verdict.redirect"),
        150,
    )
    .unwrap();
    assert_eq!(attempt_open(&db, &id, 200, None).unwrap(), 2);
    // A different step execution numbers independently.
    assert_eq!(attempt_open(&db, &sid("se-2"), 200, None).unwrap(), 1);
}

#[test]
fn close_persists_the_attempts_own_outcome() {
    let db = db();
    let id = sid("se-1");
    let no = attempt_open(&db, &id, 100, None).unwrap();
    attempt_close(
        &db,
        &id,
        no,
        "failed",
        0.42,
        1234,
        6500,
        Some("environment"),
        Some("error: boom\nat <WT>/src/lib.rs"),
        Some("environment.in_place"),
        300,
    )
    .unwrap();

    let rows = attempts_for_step(&db, &id).unwrap();
    assert_eq!(rows.len(), 1);
    let a = &rows[0];
    assert_eq!(a.attempt_no, 1);
    assert_eq!(a.status, "failed");
    assert!((a.cost_usd.unwrap() - 0.42).abs() < f64::EPSILON);
    assert_eq!(a.tokens, Some(1234));
    assert_eq!(a.wall_clock_ms, Some(6500));
    assert_eq!(a.error_class.as_deref(), Some("environment"));
    assert_eq!(
        a.failure_fingerprint.as_deref(),
        Some("error: boom\nat <WT>/src/lib.rs")
    );
    assert_eq!(a.applied_rule.as_deref(), Some("environment.in_place"));
    assert_eq!(a.started_at, 100);
    assert_eq!(a.ended_at, Some(300));
}

/// A retry loop leaves one row per dispatch, ordered, with each row's own
/// classification — history, not a single overwritten slot.
#[test]
fn a_retry_history_keeps_every_attempts_class() {
    let db = db();
    let id = sid("se-1");
    for (class, cost) in [("verdict", 0.10), ("verdict", 0.20), ("environment", 0.05)] {
        let no = attempt_open(&db, &id, 100 * no_hint(&db, &id), None).unwrap();
        attempt_close(
            &db,
            &id,
            no,
            "failed",
            cost,
            1,
            10,
            Some(class),
            None,
            None,
            999,
        )
        .unwrap();
    }
    let rows = attempts_for_step(&db, &id).unwrap();
    assert_eq!(
        rows.iter()
            .map(|a| (a.attempt_no, a.error_class.clone().unwrap()))
            .collect::<Vec<_>>(),
        vec![
            (1, "verdict".to_string()),
            (2, "verdict".to_string()),
            (3, "environment".to_string()),
        ]
    );
}

fn no_hint(db: &SqliteAdapter, id: &StepExecutionId) -> i64 {
    (attempts_for_step(db, id).unwrap().len() + 1) as i64
}

/// A crash leaves a `running` row behind; the next dispatch of the same
/// step closes it as `interrupted` instead of letting it dangle forever.
#[test]
fn reopening_interrupts_a_stale_running_attempt() {
    let db = db();
    let id = sid("se-1");
    assert_eq!(attempt_open(&db, &id, 100, None).unwrap(), 1);
    // No close — simulate a killed process. Re-dispatch:
    assert_eq!(attempt_open(&db, &id, 500, None).unwrap(), 2);

    let rows = attempts_for_step(&db, &id).unwrap();
    assert_eq!(rows[0].status, "interrupted");
    assert_eq!(rows[0].ended_at, Some(500));
    assert_eq!(rows[1].status, "running");
    assert_eq!(rows[1].ended_at, None);
}

/// P1.14: the workspace fingerprint recorded at open rides on the row,
/// and the idempotency key is derived as `<se_id>#<attempt_no>#<fp>` —
/// `unknown` standing in when the probe failed.
#[test]
fn open_records_fingerprint_and_idempotency_key() {
    let db = db();
    let id = sid("se-1");
    let fp = "0123456789abcdef0123456789abcdef01234567:clean";
    assert_eq!(attempt_open(&db, &id, 100, Some(fp)).unwrap(), 1);
    assert_eq!(attempt_open(&db, &id, 200, None).unwrap(), 2);

    let rows = attempts_for_step(&db, &id).unwrap();
    assert_eq!(rows[0].workspace_fingerprint.as_deref(), Some(fp));
    assert_eq!(
        rows[0].idempotency_key.as_deref(),
        Some(format!("se-1#1#{fp}").as_str())
    );
    assert_eq!(rows[1].workspace_fingerprint, None);
    assert_eq!(rows[1].idempotency_key.as_deref(), Some("se-1#2#unknown"));
}
