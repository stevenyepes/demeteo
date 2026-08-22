// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/ticket.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::attachment::AttachedFile;
use crate::domain::ids::WorkflowId;
use crate::ports::discovery::DiscoveryPort;
use rusqlite::Connection;

/// Seeded with the project and Discovery rows because foreign keys are enforced
/// here: a Ticket cascades off `discoveries`, which cascades off `projects`.
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
         VALUES ('d-1', 'p-1', 'plan', 'open', 'local', 'claude-code', 0, 0)",
        [],
    )
    .unwrap();
    drop(conn);
    db
}

fn did() -> DiscoveryId {
    DiscoveryId::from("d-1".to_string())
}

fn tid(id: &str) -> TicketId {
    TicketId::from(id.to_string())
}

fn attachment() -> AttachedFile {
    AttachedFile {
        id: "att-1".to_string(),
        name: "mock.png".to_string(),
        mime: "image/png".to_string(),
        sha256: "a".repeat(64),
        size: 1234,
        source_filename: "Screenshot 2026-08-21.png".to_string(),
    }
}

/// The optional TEXT columns are given values that would each be a legal answer
/// for their neighbours — a test command, a workflow id, an agent kind, a model
/// — so a positional slip in the row mapper fails an assertion instead of
/// producing a plausible ticket.
fn ticket(id: &str, seq: i64) -> Ticket {
    Ticket {
        id: tid(id),
        discovery_id: did(),
        seq,
        title: "carve out the port".to_string(),
        description: "the long version".to_string(),
        acceptance: vec!["it compiles".to_string(), "it round-trips".to_string()],
        files: vec!["src/ports/discovery.rs".to_string()],
        blocked_by: vec![tid("t-0")],
        test_command: Some("npm run checks:code".to_string()),
        workflow_id: Some(WorkflowId::from("wf_starter_feature".to_string())),
        agent_kind: Some("opencode".to_string()),
        model: Some("sonnet".to_string()),
        effort: Some(EffortLevel::Max),
        attachments: vec![attachment()],
        state: TicketState::Unstarted,
        drop_reason: None,
        force_start_reason: None,
        force_started_at: None,
        feature_id: None,
        created_at: 100,
        updated_at: 100,
    }
}

#[test]
fn a_ticket_round_trips_every_column() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1)]).unwrap();

    let read = TicketPort::get(&db, &tid("t-1")).unwrap().unwrap();
    assert_eq!(read.discovery_id, did());
    assert_eq!(read.seq, 1);
    assert_eq!(read.title, "carve out the port");
    assert_eq!(read.description, "the long version");
    assert_eq!(read.acceptance, vec!["it compiles", "it round-trips"]);
    assert_eq!(read.files, vec!["src/ports/discovery.rs"]);
    assert_eq!(read.blocked_by, vec![tid("t-0")]);
    assert_eq!(read.test_command.as_deref(), Some("npm run checks:code"));
    assert_eq!(
        read.workflow_id,
        Some(WorkflowId::from("wf_starter_feature".to_string()))
    );
    assert_eq!(read.agent_kind.as_deref(), Some("opencode"));
    assert_eq!(read.model.as_deref(), Some("sonnet"));
    assert_eq!(read.effort, Some(EffortLevel::Max));
    assert_eq!(read.attachments, vec![attachment()]);
    assert_eq!(read.state, TicketState::Unstarted);
    assert_eq!(read.drop_reason, None);
    assert_eq!(read.force_start_reason, None);
    assert_eq!(read.force_started_at, None);
    assert_eq!(read.feature_id, None);
    assert_eq!(read.created_at, 100);
    assert_eq!(read.updated_at, 100);
}

/// Re-running decomposition writes the set it proposes and must leave every
/// row it did not mention alone — §5.3's rule is that started tickets are
/// immutable, and a batch that deleted its complement would take them with it.
#[test]
fn a_batch_replaces_what_it_names_and_leaves_the_rest() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1), ticket("t-2", 2)])
        .unwrap();
    db.upsert_batch(&[Ticket {
        title: "revised".to_string(),
        ..ticket("t-1", 1)
    }])
    .unwrap();

    let all = db.list_for_discovery(&did()).unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[0].title, "revised");
    assert_eq!(all[1].title, "carve out the port");
    assert_eq!(all[0].seq, 1, "the list is in seq order");
    assert_eq!(all[1].seq, 2);
}

/// A number is never reissued once a ticket in the middle is removed: a user
/// saying "ticket 2" must not find two of them.
#[test]
fn the_next_seq_is_one_past_the_highest_not_one_past_the_count() {
    let db = db();
    assert_eq!(db.next_seq(&did()).unwrap(), 1);

    db.upsert_batch(&[ticket("t-1", 1), ticket("t-2", 2), ticket("t-3", 3)])
        .unwrap();
    assert_eq!(db.next_seq(&did()).unwrap(), 4);

    TicketPort::delete(&db, &tid("t-2")).unwrap();
    assert_eq!(db.next_seq(&did()).unwrap(), 4);
}

/// The three patch behaviours the `Option<Option<T>>` shape exists for, on the
/// columns a lifecycle transition actually writes: starting attaches a feature
/// id, and a cancel-and-restart has to be able to clear it back to nothing.
/// The JSON-backed lists take `Some(vec![])` as their clear.
#[test]
fn a_patch_distinguishes_leaving_alone_from_clearing() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1)]).unwrap();

    TicketPort::update(
        &db,
        &tid("t-1"),
        &TicketPatch {
            state: Some(TicketState::Started),
            feature_id: Some(Some(FeatureId::from("f-1".to_string()))),
            force_start_reason: Some(Some("no forge remote".to_string())),
            force_started_at: Some(Some(250)),
            ..Default::default()
        },
        200,
    )
    .unwrap();
    let read = TicketPort::get(&db, &tid("t-1")).unwrap().unwrap();
    assert_eq!(read.state, TicketState::Started);
    assert_eq!(read.feature_id, Some(FeatureId::from("f-1".to_string())));
    assert_eq!(read.force_start_reason.as_deref(), Some("no forge remote"));
    assert_eq!(read.force_started_at, Some(250));
    assert_eq!(
        read.blocked_by,
        vec![tid("t-0")],
        "an unmentioned list stands"
    );
    assert_eq!(read.title, "carve out the port");
    assert_eq!(read.updated_at, 200);

    TicketPort::update(
        &db,
        &tid("t-1"),
        &TicketPatch {
            feature_id: Some(None),
            blocked_by: Some(Vec::new()),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    let read = TicketPort::get(&db, &tid("t-1")).unwrap().unwrap();
    assert_eq!(read.feature_id, None);
    assert!(read.blocked_by.is_empty());
    assert_eq!(
        read.state,
        TicketState::Started,
        "an unmentioned state stands"
    );
    assert_eq!(read.force_start_reason.as_deref(), Some("no forge remote"));
}

/// How the `mr_monitor` poll gets from a PR transition back to the graph it
/// unblocks. A ticket that has not started, or one whose attempt was cleared,
/// must not answer for a feature it does not name.
#[test]
fn a_feature_finds_the_tickets_that_name_it() {
    let db = db();
    db.upsert_batch(&[
        Ticket {
            state: TicketState::Started,
            feature_id: Some(FeatureId::from("f-1".to_string())),
            ..ticket("t-1", 1)
        },
        ticket("t-2", 2),
    ])
    .unwrap();

    let found = db.for_feature(&FeatureId::from("f-1".to_string())).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, tid("t-1"));
    assert!(db
        .for_feature(&FeatureId::from("f-2".to_string()))
        .unwrap()
        .is_empty());
}

/// The audit §7.1 asks for: a superseded attempt is retained and marked, not
/// overwritten, and re-recording the attempt a ticket is already on must not
/// move its `started_at` or reopen one that was closed.
#[test]
fn superseded_attempts_are_kept_and_marked() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1)]).unwrap();
    let f1 = FeatureId::from("f-1".to_string());
    let f2 = FeatureId::from("f-2".to_string());

    db.record_attempt(&tid("t-1"), &f1, 100).unwrap();
    db.record_attempt(&tid("t-1"), &f1, 999).unwrap();
    let attempts = db.list_attempts(&tid("t-1")).unwrap();
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].started_at, 100);
    assert_eq!(attempts[0].superseded_at, None);

    db.supersede_attempts(&tid("t-1"), 200).unwrap();
    db.record_attempt(&tid("t-1"), &f2, 300).unwrap();
    let attempts = db.list_attempts(&tid("t-1")).unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0].feature_id, f1);
    assert_eq!(attempts[0].superseded_at, Some(200));
    assert_eq!(attempts[1].feature_id, f2);
    assert_eq!(attempts[1].started_at, 300);
    assert_eq!(attempts[1].superseded_at, None);

    db.record_attempt(&tid("t-1"), &f1, 400).unwrap();
    assert_eq!(
        db.list_attempts(&tid("t-1")).unwrap()[0].superseded_at,
        Some(200),
        "a closed attempt must not be reopened by re-recording it"
    );
}

/// A state this build cannot name reads as started: that is the one of the
/// three it is safe to be wrong about, because it holds the row immutable and
/// releases nothing. `dropped` would satisfy every dependent in the graph.
#[test]
fn a_state_written_by_a_newer_build_degrades_to_started() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1)]).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("UPDATE tickets SET state = 'deferred'", [])
            .unwrap();
    }
    assert_eq!(
        TicketPort::get(&db, &tid("t-1")).unwrap().unwrap().state,
        TicketState::Started
    );
}

/// A JSON column a newer build wrote in a shape this one cannot read degrades
/// to empty rather than failing the whole row — the ticket's title and edges
/// are still worth showing.
#[test]
fn an_unreadable_json_column_degrades_to_empty() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1)]).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "UPDATE tickets SET acceptance_json = '{\"criteria\":[]}', attachments_json = NULL",
            [],
        )
        .unwrap();
    }
    let read = TicketPort::get(&db, &tid("t-1")).unwrap().unwrap();
    assert!(read.acceptance.is_empty());
    assert!(read.attachments.is_empty());
    assert_eq!(read.blocked_by, vec![tid("t-0")]);
    assert_eq!(read.title, "carve out the port");
}

/// Deleting an eligible Discovery takes its tickets and their attempt history
/// with it (§8.4).
#[test]
fn deleting_a_discovery_takes_its_tickets_with_it() {
    let db = db();
    db.upsert_batch(&[ticket("t-1", 1)]).unwrap();
    db.record_attempt(&tid("t-1"), &FeatureId::from("f-1".to_string()), 100)
        .unwrap();

    DiscoveryPort::delete(&db, &did()).unwrap();
    assert!(TicketPort::get(&db, &tid("t-1")).unwrap().is_none());
    assert!(db.list_attempts(&tid("t-1")).unwrap().is_empty());
}

/// The other half of the same constraint: a Ticket in a Discovery that does
/// not exist is not a Ticket.
#[test]
fn a_ticket_for_a_discovery_that_does_not_exist_is_refused() {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    assert!(db.upsert_batch(&[ticket("t-1", 1)]).is_err());
}

/// `tickets.feature_id` carries no foreign key, deliberately: a feature is
/// soft-deleted to `status = 'deleted'` and its row stays, so a cascade could
/// never fire. The ticket keeps naming the run whether or not the features
/// table has anything to say about it.
#[test]
fn a_feature_id_needs_no_features_row() {
    let db = db();
    db.upsert_batch(&[Ticket {
        state: TicketState::Started,
        feature_id: Some(FeatureId::from("f-never-existed".to_string())),
        ..ticket("t-1", 1)
    }])
    .unwrap();
    assert_eq!(
        TicketPort::get(&db, &tid("t-1"))
            .unwrap()
            .unwrap()
            .feature_id,
        Some(FeatureId::from("f-never-existed".to_string()))
    );
}
