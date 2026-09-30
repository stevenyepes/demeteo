use super::*;

use std::collections::HashMap;

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;

/// Answers only the runs it was scripted for, and records every call.
#[derive(Default)]
struct ScriptedRunner {
    answers: HashMap<String, Result<(), String>>,
    calls: Mutex<Vec<(String, String, CacheReleaseReason)>>,
}

impl ScriptedRunner {
    fn answering(answers: &[(&str, Result<(), &str>)]) -> Self {
        Self {
            answers: answers
                .iter()
                .map(|(run, answer)| (run.to_string(), answer.map_err(str::to_string)))
                .collect(),
            calls: Mutex::default(),
        }
    }

    fn calls(&self) -> Vec<(String, String, CacheReleaseReason)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl RunnerCachePort for ScriptedRunner {
    async fn release_feature_cache(
        &self,
        machine_id: &str,
        run_id: &str,
        reason: CacheReleaseReason,
    ) -> Result<(), String> {
        self.calls
            .lock()
            .unwrap()
            .push((machine_id.to_string(), run_id.to_string(), reason));
        self.answers
            .get(run_id)
            .cloned()
            .unwrap_or_else(|| Err(format!("unscripted release for run {run_id}")))
    }
}

fn settings() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn entry(run_id: &str, reason: CacheReleaseReason) -> PendingRunnerRelease {
    PendingRunnerRelease {
        machine_id: "m-1".to_string(),
        run_id: run_id.to_string(),
        reason,
    }
}

#[tokio::test]
async fn a_delivered_release_leaves_nothing_pending() {
    let runner = ScriptedRunner::answering(&[("run-1", Ok(()))]);
    let settings = settings();

    release_on_runner(
        &runner,
        &settings,
        entry("run-1", CacheReleaseReason::Merged),
    )
    .await
    .expect("the runner took it");

    assert_eq!(
        runner.calls(),
        [(
            "m-1".to_string(),
            "run-1".to_string(),
            CacheReleaseReason::Merged
        )]
    );
    assert!(read_pending(&settings).unwrap().is_empty());
}

#[tokio::test]
async fn an_unreachable_runner_keeps_the_release_for_later() {
    let runner = ScriptedRunner::answering(&[("run-1", Err("Timed out waiting on socket"))]);
    let settings = settings();

    let error = release_on_runner(
        &runner,
        &settings,
        entry("run-1", CacheReleaseReason::Dismissed),
    )
    .await
    .expect_err("the cache is still on the runner");

    assert!(error.contains("Timed out waiting on socket"), "{error}");
    assert_eq!(
        read_pending(&settings).unwrap(),
        [entry("run-1", CacheReleaseReason::Dismissed)]
    );
}

#[tokio::test]
async fn a_run_the_runner_disowns_is_not_kept() {
    let runner = ScriptedRunner::answering(&[("run-1", Err("no such run: run-1"))]);
    let settings = settings();

    release_on_runner(
        &runner,
        &settings,
        entry("run-1", CacheReleaseReason::Merged),
    )
    .await
    .expect_err("the runner refused");

    assert!(
        read_pending(&settings).unwrap().is_empty(),
        "no later try can succeed, so keeping it would resend it forever"
    );
}

#[tokio::test]
async fn a_retry_drops_what_landed_or_never_will_and_keeps_the_rest() {
    let settings = settings();
    let offline = ScriptedRunner::answering(&[
        ("landed", Err("Timed out waiting on socket")),
        ("gone", Err("Timed out waiting on socket")),
        ("old", Err("Timed out waiting on socket")),
    ]);
    for run in ["landed", "gone", "old"] {
        let _ =
            release_on_runner(&offline, &settings, entry(run, CacheReleaseReason::Merged)).await;
    }

    let back = ScriptedRunner::answering(&[
        ("landed", Ok(())),
        ("gone", Err("no such run: gone")),
        ("old", Err("unknown method: release_feature_cache")),
    ]);
    retry_pending_runner_releases(&back, &settings)
        .await
        .expect("the queue is readable");

    assert_eq!(back.calls().len(), 3);
    assert_eq!(
        read_pending(&settings).unwrap(),
        [entry("old", CacheReleaseReason::Merged)],
        "a runner too old for the method may be upgraded; keep that one"
    );
}

#[tokio::test]
async fn an_empty_queue_calls_no_runner() {
    let runner = ScriptedRunner::default();
    retry_pending_runner_releases(&runner, &settings())
        .await
        .unwrap();
    assert!(runner.calls().is_empty());
}
