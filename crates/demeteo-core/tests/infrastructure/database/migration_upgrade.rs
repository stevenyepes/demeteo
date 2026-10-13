//! Opening a database whose `refinery_schema_history` was written by an
//! *older* refinery must not be a hard failure.
//!
//! refinery records a checksum per applied migration and, by default, aborts
//! when a stored checksum disagrees with the embedded migration — the
//! "divergent migration" error. That default is wrong for a shipped desktop
//! app: the disagreement is between two library versions, not between two
//! schemas, and aborting turns a dependency bump into an app that refuses to
//! start against every database already on disk. `migration::run` therefore
//! sets `set_abort_divergent(false)`.
//!
//! Nothing in a fresh-database test can catch a regression here, because a
//! fresh database has no prior history to disagree with. These tests migrate
//! for real and then rewrite the recorded checksums, which is what a database
//! carried across a refinery upgrade actually looks like: the schema is
//! present and correct, only the bookkeeping disagrees.

use crate::adapters::database::{migration, SqliteAdapter};
use rusqlite::Connection;

/// Replace every recorded checksum with one no embedded migration can
/// produce — the worst case an upgrade could present, and the one that trips
/// refinery's default abort-on-divergent behaviour.
fn forge_foreign_checksums(conn: &Connection) {
    let rewritten = conn
        .execute(
            "UPDATE refinery_schema_history SET checksum = '0000000000000000000'",
            [],
        )
        .unwrap();
    assert!(
        rewritten > 0,
        "no history rows to diverge — the migration run recorded nothing"
    );
}

fn applied_versions(conn: &Connection) -> Vec<i32> {
    conn.prepare("SELECT version FROM refinery_schema_history ORDER BY version ASC")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}

#[test]
fn a_history_of_foreign_checksums_does_not_block_startup() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::run(&mut conn).unwrap();
    let before = applied_versions(&conn);

    forge_foreign_checksums(&conn);

    migration::run(&mut conn).expect("divergent stored checksums must not abort the migration run");
    assert_eq!(
        before,
        applied_versions(&conn),
        "a divergent history must not re-apply or drop versions"
    );
}

#[test]
fn the_schema_survives_a_run_over_a_divergent_history() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::run(&mut conn).unwrap();
    forge_foreign_checksums(&conn);
    migration::run(&mut conn).unwrap();

    // A table from the tail of the migration chain: proves the second run
    // carried the schema through rather than stopping at the first
    // disagreement and leaving a half-built database.
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'step_executions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "step_executions missing after the divergent run");

    // And the current tail of the chain, which is the half a divergent run can
    // actually stop short of.
    let count: i64 = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'sync_sessions'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1, "sync_sessions missing after the divergent run");
}

#[test]
fn the_adapter_opens_against_a_divergent_history() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::run(&mut conn).unwrap();
    forge_foreign_checksums(&conn);

    // The full adapter path, not just the migration runner: this is what the
    // app actually does on launch.
    SqliteAdapter::new(conn).expect("adapter must open a database migrated by an older refinery");
}

/// V61 runs over tickets V60 left NULL. Each one ran on the project's own
/// compute before the upgrade, so it must still resolve there afterwards —
/// not inherit a detached default from a Discovery opened on another machine
/// — and each recorded attempt must say where it went.
#[test]
fn v61_keeps_existing_tickets_local_and_places_their_attempts() {
    let mut conn = Connection::open_in_memory().unwrap();
    migration::migrations::runner()
        .set_target(refinery::Target::Version(60))
        .run(&mut conn)
        .unwrap();
    conn.execute_batch(
        "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0);
         INSERT INTO discoveries
             (id, project_id, title, status, machine_id, agent_kind, created_at, updated_at)
         VALUES ('d-1', 'p-1', 'plan', 'open', 'm-1', 'claude-code', 0, 0);
         INSERT INTO tickets (id, discovery_id, seq, title, state, created_at, updated_at)
         VALUES ('t-1', 'd-1', 1, 'unstarted', 'unstarted', 0, 0),
                ('t-2', 'd-1', 2, 'started local', 'started', 0, 0),
                ('t-3', 'd-1', 3, 'started detached', 'started', 0, 0);
         INSERT INTO tickets (id, discovery_id, seq, title, state, machine_id, created_at, updated_at)
         VALUES ('t-4', 'd-1', 4, 'chosen', 'unstarted', 'm-9', 0, 0);
         INSERT INTO ticket_feature_attempts (ticket_id, feature_id, started_at)
         VALUES ('t-2', 'f-2', 0), ('t-3', 'f-3', 0);
         INSERT INTO remote_run_mirror
             (machine_id, run_id, title, feature_id, created_at, updated_at)
         VALUES ('m-3', 'r-old', 'x', 'f-3', 1, 1), ('m-4', 'r-new', 'x', 'f-3', 2, 2);",
    )
    .unwrap();

    migration::run(&mut conn).unwrap();

    let column = |sql: &str| -> Vec<(String, Option<String>)> {
        conn.prepare(sql)
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    };
    let some = |s: &str| Some(s.to_string());
    assert_eq!(
        column("SELECT id, machine_id FROM tickets ORDER BY seq"),
        vec![
            ("t-1".to_string(), some("local")),
            ("t-2".to_string(), some("local")),
            ("t-3".to_string(), some("local")),
            ("t-4".to_string(), some("m-9")),
        ],
        "a NULL placement is backfilled local; a stored choice is kept"
    );
    assert_eq!(
        column("SELECT feature_id, machine_id FROM ticket_feature_attempts ORDER BY feature_id"),
        vec![
            ("f-2".to_string(), some("local")),
            ("f-3".to_string(), some("m-4")),
        ],
        "an attempt is placed by its latest-submitted mirror row, else local"
    );
}
