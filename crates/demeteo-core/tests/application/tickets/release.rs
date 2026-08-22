// Tests extracted from `crates/demeteo-core/src/application/tickets/release.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::{DiscoveryId, FeatureId, ProjectId, TicketId};
use crate::domain::models::TicketState;
use crate::ports::db::FeatureRepository;

/// Counts the live half of the announcement, so a test can tell "wrote a row"
/// from "told the user" — the two are the pair `already_announced` guards.
#[derive(Default)]
struct CountingNotif {
    startable: AtomicUsize,
}

impl NotificationPort for CountingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        if matches!(event, DomainEvent::TicketsStartable { .. }) {
            self.startable.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

fn db() -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO discoveries
             (id, project_id, title, status, machine_id, agent_kind, created_at, updated_at)
         VALUES ('d-1', 'p-1', 'multi-client runner', 'open', 'local', 'claude-code', 0, 0)",
        [],
    )
    .unwrap();
    drop(conn);
    db
}

fn ticket(id: &str, seq: i64, title: &str) -> Ticket {
    Ticket {
        id: TicketId::from(id.to_string()),
        discovery_id: DiscoveryId::from("d-1".to_string()),
        seq,
        title: title.to_string(),
        description: String::new(),
        acceptance: Vec::new(),
        files: Vec::new(),
        blocked_by: Vec::new(),
        test_command: None,
        workflow_id: None,
        agent_kind: None,
        model: None,
        effort: None,
        attachments: Vec::new(),
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 0,
        updated_at: 0,
    }
}

/// The prerequisite's Feature, already in its terminal state — the hook runs
/// after the poll persisted the transition, so anything it derives must be
/// derived from the stored row and not from the argument.
fn merged_feature(db: &SqliteAdapter, id: &str, mr_state: &str) -> Feature {
    let feature = Feature {
        id: FeatureId::from(id.to_string()),
        project_id: ProjectId::from("p-1".to_string()),
        workflow_id: None,
        workflow_version_id: None,
        title: "the registry".to_string(),
        description: String::new(),
        status: "completed".to_string(),
        total_cost: 0.0,
        tokens: 0,
        duration: "0s".to_string(),
        created_at: 0,
        agent_kind: None,
        model: None,
        effort: None,
        mr_url: Some("https://github.com/o/r/pull/1".to_string()),
        mr_state: Some(mr_state.to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
        harness_baseline: None,
        origin: FeatureOrigin::DefaultBranch,
        diff_base_branch: None,
        resolved_branch: None,
    };
    FeatureRepository::add(db, feature.clone()).unwrap();
    feature
}

fn startable_rows(db: &SqliteAdapter) -> Vec<Notification> {
    NotificationRepository::list(db, Some(&ProjectId::from("p-1".to_string())), u32::MAX)
        .unwrap()
        .into_iter()
        .filter(|n| n.kind == NotificationKind::TicketsStartable)
        .collect()
}

/// Seeds a prerequisite that has just merged and one ticket waiting on it.
fn merge_releases_one(db: &SqliteAdapter) -> Feature {
    let feature = merged_feature(db, "f-1", "merged");
    let mut prerequisite = ticket("t-1", 1, "the registry");
    prerequisite.state = TicketState::Started;
    prerequisite.feature_id = Some(feature.id.clone());
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-1".to_string())];
    TicketPort::upsert_batch(db, &[prerequisite, dependent]).unwrap();
    feature
}

/// §6.3 leans on this hook for the one thing a derived readiness cannot do:
/// tell a user who is not looking at the board.
#[test]
fn a_merge_that_releases_a_ticket_announces_it_once() {
    let db = db();
    let feature = merge_releases_one(&db);
    let notif = CountingNotif::default();

    release_dependents(&feature, &db, &db, &db, &db, &notif).unwrap();

    let rows = startable_rows(&db);
    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].message.contains("#2 the multiplexer"),
        "{}",
        rows[0].message
    );
    assert!(
        rows[0].message.contains("multi-client runner"),
        "{}",
        rows[0].message
    );
    assert_eq!(
        rows[0].feature_url.as_deref(),
        Some("/projects/p-1/discoveries/d-1")
    );
    assert_eq!(notif.startable.load(Ordering::SeqCst), 1);
}

/// The chosen hazard: the recompute is free to run again, so what has to be
/// idempotent is the write. A second tick over the same transition must not
/// tell the user twice.
#[test]
fn a_second_recompute_over_the_same_transition_stays_silent() {
    let db = db();
    let feature = merge_releases_one(&db);
    let notif = CountingNotif::default();

    release_dependents(&feature, &db, &db, &db, &db, &notif).unwrap();
    release_dependents(&feature, &db, &db, &db, &db, &notif).unwrap();

    assert_eq!(startable_rows(&db).len(), 1);
    assert_eq!(notif.startable.load(Ordering::SeqCst), 1);
}

/// §6.4 asks for the moment a ticket *becomes* startable. A merge that leaves
/// a second edge outstanding has released nothing, and a notice here would
/// train the user to ignore the next one.
#[test]
fn a_merge_that_releases_nothing_says_nothing() {
    let db = db();
    let feature = merged_feature(&db, "f-1", "merged");
    let mut prerequisite = ticket("t-1", 1, "the registry");
    prerequisite.state = TicketState::Started;
    prerequisite.feature_id = Some(feature.id.clone());
    let mut dependent = ticket("t-3", 3, "conformance");
    dependent.blocked_by = vec![
        TicketId::from("t-1".to_string()),
        TicketId::from("t-2".to_string()),
    ];
    TicketPort::upsert_batch(
        &db,
        &[prerequisite, ticket("t-2", 2, "the keypair"), dependent],
    )
    .unwrap();
    let notif = CountingNotif::default();

    release_dependents(&feature, &db, &db, &db, &db, &notif).unwrap();

    assert!(startable_rows(&db).is_empty());
    assert_eq!(notif.startable.load(Ordering::SeqCst), 0);
}

/// §6.4 releases a dependent on a closed PR too, and the hook has to reach it:
/// `record_merged` never runs for `closed`.
#[test]
fn a_closed_pull_request_releases_its_dependents_as_well() {
    let db = db();
    let feature = merged_feature(&db, "f-1", "closed");
    let mut prerequisite = ticket("t-1", 1, "the registry");
    prerequisite.state = TicketState::Started;
    prerequisite.feature_id = Some(feature.id.clone());
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-1".to_string())];
    TicketPort::upsert_batch(&db, &[prerequisite, dependent]).unwrap();
    let notif = CountingNotif::default();

    release_dependents(&feature, &db, &db, &db, &db, &notif).unwrap();

    assert_eq!(startable_rows(&db).len(), 1);
}

/// A Discovery usually holds several ready tickets nobody has started. Naming
/// them again on an unrelated merge is what makes a notice worth ignoring.
#[test]
fn only_the_tickets_this_prerequisite_gated_are_named() {
    let db = db();
    let feature = merged_feature(&db, "f-1", "merged");
    let mut prerequisite = ticket("t-1", 1, "the registry");
    prerequisite.state = TicketState::Started;
    prerequisite.feature_id = Some(feature.id.clone());
    let mut dependent = ticket("t-2", 2, "the multiplexer");
    dependent.blocked_by = vec![TicketId::from("t-1".to_string())];
    let idle = ticket("t-4", 4, "the operator guide");
    TicketPort::upsert_batch(&db, &[prerequisite, dependent, idle]).unwrap();
    let notif = CountingNotif::default();

    release_dependents(&feature, &db, &db, &db, &db, &notif).unwrap();

    let rows = startable_rows(&db);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].message.contains("#2"), "{}", rows[0].message);
    assert!(!rows[0].message.contains("#4"), "{}", rows[0].message);
}
