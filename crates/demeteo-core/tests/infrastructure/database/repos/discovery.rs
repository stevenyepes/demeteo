// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/discovery.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::MachineId;
use rusqlite::Connection;

/// Seeded with the project row because foreign keys are enforced here: a
/// Discovery cascades off `projects`.
fn db() -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0)",
        [],
    )
    .unwrap();
    drop(conn);
    db
}

fn did() -> DiscoveryId {
    DiscoveryId::from("d-1".to_string())
}

/// Every TEXT field is given a value that would be a legal answer for its
/// neighbours, so a positional slip in the row mapper is a failed assertion
/// rather than a plausible-looking row.
fn discovery() -> Discovery {
    Discovery {
        id: did(),
        project_id: ProjectId::from("p-1".to_string()),
        title: "rework the sync surface".to_string(),
        status: DiscoveryStatus::Open,
        machine_id: MachineId::from("local".to_string()),
        agent_kind: "claude-code".to_string(),
        model: Some("opus".to_string()),
        effort: Some(EffortLevel::XHigh),
        resume_session_id: Some("sess-abc".to_string()),
        worktree_path: Some("/repos/demeteo_wt_discovery_d-1".to_string()),
        total_cost: 1.25,
        tokens: 4096,
        created_at: 100,
        updated_at: 100,
    }
}

#[test]
fn a_discovery_round_trips_every_column() {
    let db = db();
    db.create(&discovery()).unwrap();

    let read = DiscoveryPort::get(&db, &did()).unwrap().unwrap();
    assert_eq!(read.title, "rework the sync surface");
    assert_eq!(read.status, DiscoveryStatus::Open);
    assert_eq!(read.machine_id.as_str(), "local");
    assert_eq!(read.agent_kind, "claude-code");
    assert_eq!(read.model.as_deref(), Some("opus"));
    assert_eq!(read.effort, Some(EffortLevel::XHigh));
    assert_eq!(read.resume_session_id.as_deref(), Some("sess-abc"));
    assert_eq!(
        read.worktree_path.as_deref(),
        Some("/repos/demeteo_wt_discovery_d-1")
    );
    assert_eq!(read.total_cost, 1.25);
    assert_eq!(read.tokens, 4096);
    assert_eq!(read.created_at, 100);
    assert_eq!(read.updated_at, 100);
}

#[test]
fn an_absent_discovery_is_none_rather_than_an_error() {
    let db = db();
    assert!(DiscoveryPort::get(&db, &did()).unwrap().is_none());
}

/// The three patch behaviours the `Option<Option<T>>` shape exists for, on the
/// two columns whose clear is load-bearing: a resume id that stopped resolving
/// must become NULL, and so must the worktree an idle reclaim removed. A patch
/// silently leaving either in place would resume against a session the harness
/// forgot, or hand out a path that is gone.
#[test]
fn a_patch_distinguishes_leaving_alone_from_clearing() {
    let db = db();
    db.create(&discovery()).unwrap();

    db.update(
        &did(),
        &DiscoveryPatch {
            title: Some("rework the sync surface, again".to_string()),
            ..Default::default()
        },
        200,
    )
    .unwrap();
    let read = DiscoveryPort::get(&db, &did()).unwrap().unwrap();
    assert_eq!(read.title, "rework the sync surface, again");
    assert_eq!(read.resume_session_id.as_deref(), Some("sess-abc"));
    assert_eq!(
        read.worktree_path.as_deref(),
        Some("/repos/demeteo_wt_discovery_d-1")
    );
    assert_eq!(read.updated_at, 200);

    db.update(
        &did(),
        &DiscoveryPatch {
            resume_session_id: Some(None),
            worktree_path: Some(None),
            status: Some(DiscoveryStatus::Closed),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    let read = DiscoveryPort::get(&db, &did()).unwrap().unwrap();
    assert_eq!(read.resume_session_id, None);
    assert_eq!(read.worktree_path, None);
    assert_eq!(read.status, DiscoveryStatus::Closed);
    assert_eq!(read.title, "rework the sync surface, again");

    db.update(
        &did(),
        &DiscoveryPatch {
            resume_session_id: Some(Some("sess-def".to_string())),
            effort: Some(Some(EffortLevel::Low)),
            ..Default::default()
        },
        400,
    )
    .unwrap();
    let read = DiscoveryPort::get(&db, &did()).unwrap().unwrap();
    assert_eq!(read.resume_session_id.as_deref(), Some("sess-def"));
    assert_eq!(read.effort, Some(EffortLevel::Low));
}

/// Spend accumulates rather than being overwritten: each turn reports only its
/// own cost, so a patch that assigned would leave the row holding whatever the
/// last turn happened to spend.
#[test]
fn spend_folds_across_turns() {
    let db = db();
    db.create(&discovery()).unwrap();

    for _ in 0..3 {
        db.update(
            &did(),
            &DiscoveryPatch {
                add_cost: 0.5,
                add_tokens: 1000,
                ..Default::default()
            },
            200,
        )
        .unwrap();
    }
    let read = DiscoveryPort::get(&db, &did()).unwrap().unwrap();
    assert_eq!(read.total_cost, 2.75);
    assert_eq!(read.tokens, 7096);

    db.update(&did(), &DiscoveryPatch::default(), 300).unwrap();
    let read = DiscoveryPort::get(&db, &did()).unwrap().unwrap();
    assert_eq!(
        read.total_cost, 2.75,
        "a patch with no spend must not re-count"
    );
    assert_eq!(read.tokens, 7096);
}

/// Closed Discoveries are listed alongside open ones — closing is soft — and
/// the most recently touched comes first, which is the order Project Home reads.
#[test]
fn a_projects_discoveries_list_most_recent_first() {
    let db = db();
    db.create(&discovery()).unwrap();
    db.create(&Discovery {
        id: DiscoveryId::from("d-2".to_string()),
        title: "second".to_string(),
        status: DiscoveryStatus::Closed,
        updated_at: 500,
        ..discovery()
    })
    .unwrap();

    let ids: Vec<String> = db
        .list_for_project(&ProjectId::from("p-1".to_string()))
        .unwrap()
        .into_iter()
        .map(|d| d.id.0)
        .collect();
    assert_eq!(ids, vec!["d-2".to_string(), "d-1".to_string()]);
}

fn message(id: &str, role: MessageRole, at: i64) -> DiscoveryMessage {
    DiscoveryMessage {
        id: id.to_string(),
        discovery_id: did(),
        role,
        content: format!("body of {id}"),
        cost_usd: match role {
            MessageRole::User => None,
            MessageRole::Assistant => Some(0.75),
        },
        tokens: match role {
            MessageRole::User => None,
            MessageRole::Assistant => Some(2048),
        },
        created_at: at,
    }
}

/// The transcript is the authority a turn re-seeds from, so its order and its
/// roles have to survive the round trip exactly. A user message's spend is
/// absent, not zero — the column is the only thing keeping "not asked" apart
/// from "measured as nothing".
#[test]
fn the_transcript_round_trips_in_order() {
    let db = db();
    db.create(&discovery()).unwrap();
    db.append_message(&message("m-2", MessageRole::Assistant, 200))
        .unwrap();
    db.append_message(&message("m-1", MessageRole::User, 100))
        .unwrap();

    let log = db.list_messages(&did()).unwrap();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].id, "m-1");
    assert_eq!(log[0].role, MessageRole::User);
    assert_eq!(log[0].content, "body of m-1");
    assert_eq!(log[0].cost_usd, None);
    assert_eq!(log[0].tokens, None);
    assert_eq!(log[1].id, "m-2");
    assert_eq!(log[1].role, MessageRole::Assistant);
    assert_eq!(log[1].content, "body of m-2");
    assert_eq!(log[1].cost_usd, Some(0.75));
    assert_eq!(log[1].tokens, Some(2048));
}

/// A status this build cannot name degrades to closed rather than panicking or
/// being believed, and a role it cannot name is read as the assistant's — never
/// as something the user said, which a re-seeded transcript would replay as an
/// instruction.
#[test]
fn vocabulary_a_newer_build_wrote_degrades() {
    let db = db();
    db.create(&discovery()).unwrap();
    db.append_message(&message("m-1", MessageRole::User, 100))
        .unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("UPDATE discoveries SET status = 'archived'", [])
            .unwrap();
        conn.execute("UPDATE discovery_messages SET role = 'system'", [])
            .unwrap();
    }

    assert_eq!(
        DiscoveryPort::get(&db, &did()).unwrap().unwrap().status,
        DiscoveryStatus::Closed
    );
    assert_eq!(
        db.list_messages(&did()).unwrap()[0].role,
        MessageRole::Assistant
    );
}

/// Deleting a Discovery takes its transcript with it — §8.4's rule is that an
/// eligible Discovery goes whole, and an orphaned message log is a conversation
/// with nothing to attach it to.
#[test]
fn deleting_a_discovery_takes_its_transcript_with_it() {
    let db = db();
    db.create(&discovery()).unwrap();
    db.append_message(&message("m-1", MessageRole::User, 100))
        .unwrap();

    DiscoveryPort::delete(&db, &did()).unwrap();
    assert!(DiscoveryPort::get(&db, &did()).unwrap().is_none());
    assert!(db.list_messages(&did()).unwrap().is_empty());
}

/// The other half of the same constraint: a Discovery in a project that does
/// not exist is not a Discovery.
#[test]
fn a_discovery_for_a_project_that_does_not_exist_is_refused() {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    assert!(db.create(&discovery()).is_err());
}
