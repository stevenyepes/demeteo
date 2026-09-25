//! A synthetic park against the real `SqliteAdapter`. The waiter is never
//! delivered to, and the cancel channel has already fired, so `park_for_human`
//! returns only what its fast path found in the row: `Some` is an answer it
//! consumed without asking, `None` is a park that would have waited.

use super::*;

use crate::adapters::database::SqliteAdapter;
use rusqlite::Connection;

const SE: &str = "se-f-1-s-implement";

struct NoopNotif;

impl NotificationPort for NoopNotif {
    fn emit(&self, _: &DomainEvent) -> Result<(), String> {
        Ok(())
    }
}

fn gates() -> SqliteAdapter {
    let sqlite = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    {
        let conn = sqlite.conn.lock().unwrap();
        conn.execute_batch(&format!(
            "INSERT INTO projects (id, name, created_at) VALUES ('p-1', 'demeteo', 0);
             INSERT INTO features (id, project_id, title, created_at)
             VALUES ('f-1', 'p-1', 'interrupted twice', 0);
             INSERT INTO step_executions
                 (id, feature_id, step_id, step_index, step_kind, status, created_at, updated_at)
             VALUES ('{SE}', 'f-1', 's-implement', 0, 'agent', 'interrupted', 0, 0);"
        ))
        .unwrap();
    }
    sqlite
}

fn se() -> StepExecutionId {
    StepExecutionId::from(SE.to_string())
}

async fn park_already_cancelled(gates: &SqliteAdapter) -> Option<GateDecision> {
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();
    let waiters = Arc::new(Mutex::new(HashMap::new()));
    let f_id = FeatureId::from("f-1".to_string());
    park_for_human(
        SyntheticGate {
            gates,
            notif: &NoopNotif,
            waiters: &waiters,
            f_id: &f_id,
        },
        &se(),
        rx,
    )
    .await
}

#[tokio::test]
async fn a_second_restart_does_not_resume_on_the_first_restarts_approval() {
    let gates = gates();
    pose_synthetic_gate(&gates, &se()).unwrap();
    gates.upsert_decision(&se(), "approve", None, 1).unwrap();

    pose_synthetic_gate(&gates, &se()).unwrap();

    assert!(
        park_already_cancelled(&gates).await.is_none(),
        "the park consumed an approval given to an earlier question"
    );
}

#[tokio::test]
async fn an_answer_to_this_restarts_question_is_consumed_without_waiting() {
    let gates = gates();
    pose_synthetic_gate(&gates, &se()).unwrap();
    gates.upsert_decision(&se(), "approve", None, 1).unwrap();

    let decision = park_already_cancelled(&gates).await;

    assert_eq!(
        decision.and_then(|d| d.decision).as_deref(),
        Some("approve")
    );
    assert!(gates.latest_for_step(&se()).unwrap().is_none());
}

/// A zero-ticket park survives a restart with its own row, the human answers
/// it, and the step is dispatched again: dispatch leaves that answer for the
/// re-run's identical park to read.
#[tokio::test]
async fn a_surviving_parks_answer_reaches_the_re_park() {
    let gates = gates();
    gates
        .create(undecided_row(park_question_id(&se()), &se()))
        .unwrap();
    gates
        .upsert_decision(&se(), "redirect", Some("split the migration"), 1)
        .unwrap();

    void_resume_question(&gates, &se()).unwrap();

    let decision = park_already_cancelled(&gates)
        .await
        .expect("the answer to the park that survived the restart was discarded");
    assert_eq!(decision.decision.as_deref(), Some("redirect"));
    assert_eq!(decision.feedback.as_deref(), Some("split the migration"));
}

/// The watchdog's resume question, answered after the guard already let the
/// node run, must not answer a different park later in that run.
#[tokio::test]
async fn an_unread_resume_answer_does_not_answer_a_later_park() {
    let gates = gates();
    pose_synthetic_gate(&gates, &se()).unwrap();
    gates.upsert_decision(&se(), "approve", None, 1).unwrap();

    void_resume_question(&gates, &se()).unwrap();

    assert!(
        park_already_cancelled(&gates).await.is_none(),
        "a later park consumed the answer to the watchdog's resume question"
    );
}
