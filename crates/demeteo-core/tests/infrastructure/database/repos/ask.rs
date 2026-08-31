// Tests extracted from `crates/demeteo-core/src/adapters/database/repos/ask.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::MachineId;
use rusqlite::Connection;

/// Seeded with the project row because foreign keys are enforced here: an
/// Ask thread cascades off `projects`.
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

fn tid() -> AskThreadId {
    AskThreadId::from("t-1".to_string())
}

/// Every TEXT column gets a value that would also pass as one of its
/// neighbours', so a positional slip in `row_to_thread` fails an assertion
/// instead of producing a row that merely looks right.
fn thread() -> AskThread {
    AskThread {
        id: tid(),
        project_id: ProjectId::from("p-1".to_string()),
        title: "quick question".to_string(),
        status: AskStatus::Open,
        agent_kind: "claude-code".to_string(),
        model: Some("opus".to_string()),
        effort: Some(EffortLevel::XHigh),
        machine_id: MachineId::from("local".to_string()),
        worktree_path: Some("/repos/demeteo_wt_ask_t-1".to_string()),
        session_id: Some("sess-abc".to_string()),
        turn_count: 0,
        cost_usd: 0.0,
        tokens: 0,
        network: true,
        created_at: 100,
        updated_at: 100,
    }
}

fn assistant_activity() -> TurnActivity {
    TurnActivity {
        reads: 6,
        edits: 0,
        writes: 0,
        ran: 9,
        commands: vec!["git log --oneline".to_string(), "rg ask".to_string()],
    }
}

fn message(id: &str, role: MessageRole, at: i64) -> AskMessage {
    AskMessage {
        id: id.to_string(),
        thread_id: tid(),
        role,
        text: format!("body of {id}"),
        cost_usd: match role {
            MessageRole::User => None,
            MessageRole::Assistant => Some(0.75),
        },
        tokens: match role {
            MessageRole::User => None,
            MessageRole::Assistant => Some(2048),
        },
        turn_activity: match role {
            MessageRole::User => None,
            MessageRole::Assistant => Some(assistant_activity()),
        },
        canvas_paths: None,
        checked_commit_sha: None,
        created_at: at,
    }
}

#[test]
fn a_thread_round_trips_every_column() {
    let db = db();
    db.create(&thread()).unwrap();

    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(read.title, "quick question");
    assert_eq!(read.status, AskStatus::Open);
    assert_eq!(read.agent_kind, "claude-code");
    assert_eq!(read.model.as_deref(), Some("opus"));
    assert_eq!(read.effort, Some(EffortLevel::XHigh));
    assert_eq!(read.machine_id.as_str(), "local");
    assert_eq!(
        read.worktree_path.as_deref(),
        Some("/repos/demeteo_wt_ask_t-1")
    );
    assert_eq!(read.session_id.as_deref(), Some("sess-abc"));
    assert_eq!(read.turn_count, 0);
    assert_eq!(read.cost_usd, 0.0);
    assert_eq!(read.tokens, 0);
    assert!(read.network);
    assert_eq!(read.created_at, 100);
    assert_eq!(read.updated_at, 100);
}

/// New threads default to network access on, matching today's hard-coded
/// `Access::Allow` posture. A row written before this column existed (here
/// simulated with a direct `INSERT` naming no `network` value) must default
/// to the same `true` rather than reading back as network-denied.
#[test]
fn network_column_round_trips() {
    let db = db();
    db.create(&thread()).unwrap();
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert!(read.network);

    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO ask_thread
                (id, project_id, title, status, agent_kind, machine_id,
                 turn_count, cost_usd, tokens, created_at, updated_at)
             VALUES
                ('t-pre', 'p-1', 'pre-migration', 'open', 'claude-code', 'local',
                 0, 0.0, 0, 100, 100)",
            [],
        )
        .unwrap();
    }
    let pre = AskPort::get(&db, &AskThreadId::from("t-pre".to_string()))
        .unwrap()
        .unwrap();
    assert!(
        pre.network,
        "a row with no explicit value must still default to true"
    );
}

#[test]
fn an_absent_thread_is_none_rather_than_an_error() {
    let db = db();
    assert!(AskPort::get(&db, &tid()).unwrap().is_none());
}

/// Threads are scoped to their project and ordered most-recently-touched
/// first, the order Project Home reads.
#[test]
fn a_projects_threads_list_only_its_own_most_recent_first() {
    let db = db();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO projects (id, name, created_at) VALUES ('p-2', 'other', 0)",
        [],
    )
    .unwrap();
    drop(conn);

    db.create(&thread()).unwrap();
    db.create(&AskThread {
        id: AskThreadId::from("t-2".to_string()),
        title: "second".to_string(),
        updated_at: 500,
        ..thread()
    })
    .unwrap();
    db.create(&AskThread {
        id: AskThreadId::from("t-3".to_string()),
        project_id: ProjectId::from("p-2".to_string()),
        title: "other project".to_string(),
        updated_at: 900,
        ..thread()
    })
    .unwrap();

    let ids: Vec<String> = db
        .list_for_project(&ProjectId::from("p-1".to_string()))
        .unwrap()
        .into_iter()
        .map(|t| t.id.0)
        .collect();
    assert_eq!(ids, vec!["t-2".to_string(), "t-1".to_string()]);
}

/// The transcript is the authority a turn re-seeds from, so its order and
/// roles must survive the round trip exactly — including two messages tied
/// on `created_at`, where `id` breaks the tie.
#[test]
fn the_transcript_round_trips_in_created_at_then_id_order() {
    let db = db();
    db.create(&thread()).unwrap();
    db.append_message(&message("m-3", MessageRole::Assistant, 300))
        .unwrap();
    db.append_message(&message("m-1", MessageRole::User, 100))
        .unwrap();
    // Tied timestamp with m-1's neighbour: id order decides.
    db.append_message(&message("m-2b", MessageRole::Assistant, 200))
        .unwrap();
    db.append_message(&message("m-2a", MessageRole::User, 200))
        .unwrap();

    let log = db.list_messages(&tid()).unwrap();
    let ids: Vec<&str> = log.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, vec!["m-1", "m-2a", "m-2b", "m-3"]);
}

/// Optional telemetry and activity round-trip: present for an assistant
/// turn, absent (not zeroed) for a user turn.
#[test]
fn optional_activity_and_telemetry_round_trip() {
    let db = db();
    db.create(&thread()).unwrap();
    db.append_message(&message("m-1", MessageRole::User, 100))
        .unwrap();
    db.append_message(&message("m-2", MessageRole::Assistant, 200))
        .unwrap();

    let log = db.list_messages(&tid()).unwrap();
    assert_eq!(log[0].cost_usd, None);
    assert_eq!(log[0].tokens, None);
    assert_eq!(log[0].turn_activity, None);
    assert_eq!(log[1].cost_usd, Some(0.75));
    assert_eq!(log[1].tokens, Some(2048));
    assert_eq!(log[1].turn_activity, Some(assistant_activity()));
}

/// Path verification is optional per-turn telemetry, same shape as
/// `turn_activity`: present when a turn checked its canvas paths, absent
/// otherwise — never present-but-empty for a turn that checked nothing.
#[test]
fn canvas_path_verdicts_and_checked_commit_round_trip() {
    let db = db();
    db.create(&thread()).unwrap();
    let verdicts = vec![
        CanvasPathVerdict {
            node_id: "n-1".to_string(),
            path: "src/lib.rs".to_string(),
            resolved: true,
        },
        CanvasPathVerdict {
            node_id: "n-2".to_string(),
            path: "src/missing.rs".to_string(),
            resolved: false,
        },
    ];
    db.append_message(&AskMessage {
        canvas_paths: Some(verdicts.clone()),
        checked_commit_sha: Some("deadbeef".to_string()),
        ..message("m-1", MessageRole::Assistant, 100)
    })
    .unwrap();

    let log = db.list_messages(&tid()).unwrap();
    assert_eq!(log[0].canvas_paths, Some(verdicts));
    assert_eq!(log[0].checked_commit_sha.as_deref(), Some("deadbeef"));
}

/// A message that never checked its canvas paths reads back with both
/// fields absent, mirroring `turn_activity`'s None case.
#[test]
fn absent_canvas_path_verdicts_and_checked_commit_round_trip_as_none() {
    let db = db();
    db.create(&thread()).unwrap();
    db.append_message(&message("m-1", MessageRole::Assistant, 100))
        .unwrap();

    let log = db.list_messages(&tid()).unwrap();
    assert_eq!(log[0].canvas_paths, None);
    assert_eq!(log[0].checked_commit_sha, None);
}

/// A message written before an activity summary existed reads back as no
/// summary at all, never as a turn measured to have touched nothing.
#[test]
fn a_message_stored_without_activity_reads_as_none() {
    let db = db();
    db.create(&thread()).unwrap();
    db.append_message(&message("m-1", MessageRole::Assistant, 100))
        .unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("UPDATE ask_message SET turn_activity_json = NULL", [])
            .unwrap();
    }

    let log = db.list_messages(&tid()).unwrap();
    assert_eq!(log[0].turn_activity, None);
}

/// Patch telemetry is additive: each turn reports only its own spend, so a
/// patch that assigned would leave the row holding whatever the last turn
/// happened to spend rather than the running total.
#[test]
fn patch_telemetry_accumulates_rather_than_replaces() {
    let db = db();
    db.create(&thread()).unwrap();

    for _ in 0..3 {
        db.update(
            &tid(),
            &AskThreadPatch {
                add_turns: 1,
                add_cost_usd: 0.5,
                add_tokens: 1000,
                ..Default::default()
            },
            200,
        )
        .unwrap();
    }
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(read.turn_count, 3);
    assert_eq!(read.cost_usd, 1.5);
    assert_eq!(read.tokens, 3000);
    assert_eq!(read.updated_at, 200);

    db.update(&tid(), &AskThreadPatch::default(), 300).unwrap();
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(
        read.turn_count, 3,
        "a patch with no spend must not re-count"
    );
    assert_eq!(read.cost_usd, 1.5);
    assert_eq!(read.tokens, 3000);
}

/// The three patch behaviours the `Option<Option<T>>` shape exists for: a
/// resumed session id and reclaimed worktree path must be able to clear to
/// NULL, not just get set.
#[test]
fn a_patch_distinguishes_leaving_alone_from_clearing() {
    let db = db();
    db.create(&thread()).unwrap();

    db.update(
        &tid(),
        &AskThreadPatch {
            title: Some("renamed".to_string()),
            ..Default::default()
        },
        200,
    )
    .unwrap();
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(read.title, "renamed");
    assert_eq!(read.session_id.as_deref(), Some("sess-abc"));
    assert_eq!(
        read.worktree_path.as_deref(),
        Some("/repos/demeteo_wt_ask_t-1")
    );

    db.update(
        &tid(),
        &AskThreadPatch {
            session_id: Some(None),
            worktree_path: Some(None),
            status: Some(AskStatus::Closed),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(read.session_id, None);
    assert_eq!(read.worktree_path, None);
    assert_eq!(read.status, AskStatus::Closed);
    assert_eq!(read.title, "renamed");
}

/// The same `Option<Option<T>>` contract for the run-shape columns, which
/// no surface reaches today: a thread reverting to its harness's defaults
/// clears `model` and `effort`, and nothing but `Some(None)` says that —
/// a `None` patch has to leave a configured pair standing.
#[test]
fn a_patch_clears_model_and_effort_only_when_it_says_so() {
    let db = db();
    db.create(&thread()).unwrap();

    db.update(
        &tid(),
        &AskThreadPatch {
            title: Some("still opus".to_string()),
            ..Default::default()
        },
        200,
    )
    .unwrap();
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(read.model.as_deref(), Some("opus"));
    assert_eq!(read.effort, Some(EffortLevel::XHigh));

    db.update(
        &tid(),
        &AskThreadPatch {
            model: Some(None),
            effort: Some(None),
            ..Default::default()
        },
        300,
    )
    .unwrap();
    let read = AskPort::get(&db, &tid()).unwrap().unwrap();
    assert_eq!(read.model, None);
    assert_eq!(read.effort, None);
    assert_eq!(read.title, "still opus");
}

/// Deleting a thread takes its transcript with it through the declared
/// foreign key cascade — an orphaned message log is a conversation with
/// nothing to attach it to.
#[test]
fn deleting_a_thread_removes_its_messages() {
    let db = db();
    db.create(&thread()).unwrap();
    db.append_message(&message("m-1", MessageRole::User, 100))
        .unwrap();
    db.append_message(&message("m-2", MessageRole::Assistant, 200))
        .unwrap();

    AskPort::delete(&db, &tid()).unwrap();
    assert!(AskPort::get(&db, &tid()).unwrap().is_none());
    assert!(db.list_messages(&tid()).unwrap().is_empty());
}

/// The other half of the same constraint: an Ask thread in a project that
/// does not exist is not an Ask thread.
#[test]
fn a_thread_for_a_project_that_does_not_exist_is_refused() {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    assert!(db.create(&thread()).is_err());
}
