use super::*;

use std::sync::Mutex;

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;

const FEATURE: &str = "f-1";
const BRANCH: &str = "demeteo/features/f-1";

/// Answers with one scripted result, and records every call.
struct ScriptedRunner {
    answer: Result<BranchRefreshOutcome, String>,
    calls: Mutex<Vec<(String, String, Option<String>)>>,
}

impl ScriptedRunner {
    fn answering(answer: Result<BranchRefreshOutcome, &str>) -> Self {
        Self {
            answer: answer.map_err(str::to_string),
            calls: Mutex::default(),
        }
    }
}

#[async_trait]
impl RunnerBranchPort for ScriptedRunner {
    async fn refresh_feature_branch(
        &self,
        machine_id: &str,
        run_id: &str,
        git_pat: Option<&str>,
    ) -> Result<BranchRefreshOutcome, String> {
        self.calls.lock().unwrap().push((
            machine_id.to_string(),
            run_id.to_string(),
            git_pat.map(str::to_string),
        ));
        self.answer.clone()
    }
}

#[derive(Default)]
struct Events(Mutex<Vec<DomainEvent>>);

impl NotificationPort for Events {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn db(mirrored: bool) -> SqliteAdapter {
    let db = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    if mirrored {
        db.upsert_submitted("m-1", "run-1", Some("p-1"), Some(FEATURE), "widget", 1)
            .unwrap();
    }
    db
}

async fn refresh(db: &SqliteAdapter, runner: &ScriptedRunner, events: &Events) -> Option<String> {
    let ports = BranchRefreshPorts {
        mirrors: db,
        runner,
        features: db,
        notifications: db,
        notif: events,
    };
    refresh_runner_branch(&ports, &FeatureId::from(FEATURE), BRANCH, Some("pat"))
        .await
        .expect("the bell is writable")
}

#[tokio::test]
async fn a_feature_no_runner_holds_asks_nobody() {
    let db = db(false);
    let runner = ScriptedRunner::answering(Err("must not be asked"));
    let events = Events::default();

    assert_eq!(refresh(&db, &runner, &events).await, None);
    assert!(runner.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refresh_that_landed_tells_nobody() {
    let db = db(true);
    let runner = ScriptedRunner::answering(Ok(BranchRefreshOutcome::Updated {
        from: "a".to_string(),
        to: "b".to_string(),
    }));
    let events = Events::default();

    assert_eq!(refresh(&db, &runner, &events).await, None);
    assert_eq!(
        *runner.calls.lock().unwrap(),
        [(
            "m-1".to_string(),
            "run-1".to_string(),
            Some("pat".to_string())
        )]
    );
    assert!(NotificationRepository::list(&db, None, 10)
        .unwrap()
        .is_empty());
    assert!(events.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn an_old_runner_is_reported_in_the_bell_and_live() {
    let db = db(true);
    let runner = ScriptedRunner::answering(Err("unknown method: refresh_feature_branch"));
    let events = Events::default();

    let told = refresh(&db, &runner, &events).await.expect("news");

    let rows = NotificationRepository::list(&db, None, 10).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].kind, NotificationKind::RunnerBranchStale);
    assert_eq!(rows[0].feature_id, FEATURE);
    assert_eq!(rows[0].project_id, "p-1");
    assert_eq!(rows[0].message, told);
    assert!(told.contains("too old"), "{told}");
    assert!(matches!(
        events.0.lock().unwrap().as_slice(),
        [DomainEvent::RunnerBranchStale { message, .. }] if *message == told
    ));
}

#[tokio::test]
async fn a_refusal_is_reported_with_the_runners_reason() {
    let db = db(true);
    let runner = ScriptedRunner::answering(Ok(BranchRefreshOutcome::Refused {
        reason: "the run is still running on the runner".to_string(),
    }));
    let events = Events::default();

    let told = refresh(&db, &runner, &events).await.expect("news");

    assert!(told.contains("still running"), "{told}");
    assert_eq!(
        NotificationRepository::list(&db, None, 10).unwrap().len(),
        1
    );
}
