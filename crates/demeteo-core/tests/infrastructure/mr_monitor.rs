// Tests for `crates/demeteo-core/src/adapters/mr_monitor.rs`
// (mirrored-tests convention). `super` = that module — the
// `#[path = "..."]` include in the source file under `mod tests`
// re-exports these as a private submodule, so we can name the
// otherwise-private `check_mr_states` and `record_merged` helpers
// directly.

use async_trait::async_trait;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;
use crate::domain::ids::{FeatureId, ProjectId};
use crate::domain::models::{Feature, MrInfo, Project, PublishOptions};
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::mr_publisher::MrPublisher;
use crate::ports::notification::{DomainEvent, NotificationPort};

// ── Test infrastructure ───────────────────────────────────────────────────

/// `MrPublisher` stub that always reports the MR as `"merged"` and
/// counts how many times it was consulted. Production
/// `HttpMrPublisher::fetch_mr_state` hits GitHub/GitLab — this stub
/// removes the network from the path so we can drive `check_mr_states`
/// deterministically.
#[derive(Default)]
struct StubPublisherAlwaysMerged {
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

#[async_trait]
impl MrPublisher for StubPublisherAlwaysMerged {
    async fn publish_mr(
        &self,
        _: &str,
        _: &crate::domain::ids::FeatureId,
        _: PublishOptions,
    ) -> Result<MrInfo, String> {
        unimplemented!()
    }

    async fn fetch_mr_state(&self, project_id: &str, mr_url: &str) -> Result<String, String> {
        self.calls
            .lock()
            .unwrap()
            .push((project_id.to_string(), mr_url.to_string()));
        Ok("merged".to_string())
    }
}

/// Notification port that records every emitted event so tests can
/// assert on the `MrMerged` stream. Mirrors the `CapturingNotif` in the
/// e2e suite (`tests/e2e/step_executor.rs`) — duplicated here so the
/// mr_monitor tests stay self-contained and don't pull the full
/// step_executor fixture graph.
#[derive(Default)]
struct CapturingNotif {
    events: Mutex<Vec<DomainEvent>>,
}

impl NotificationPort for CapturingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

fn open_db() -> SqliteAdapter {
    SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap()
}

fn seed_project_and_open_mr(db: &SqliteAdapter, fid: &str, mr_url: &str) -> Feature {
    let pid = ProjectId::from("p-1".to_string());
    ProjectRepository::add(
        db,
        Project {
            id: pid.clone(),
            name: "p".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    )
    .unwrap();
    let feature = Feature {
        effort: None,
        id: FeatureId::from(fid.to_string()),
        project_id: pid,
        workflow_id: None,
        title: "Repro feature".to_string(),
        description: String::new(),
        status: "completed".to_string(),
        total_cost: 0.0,
        tokens: 0,
        duration: "0s".to_string(),
        created_at: 1000,
        agent_kind: None,
        model: None,
        mr_url: Some(mr_url.to_string()),
        mr_state: Some("open".to_string()),
        pr_title: None,
        pr_body: None,
        commit_artifacts: None,
        loop_iterations: None,
        max_budget_usd: None,
        step_overrides: Vec::new(),
        attachments: Vec::new(),
    };
    FeatureRepository::add(db, feature.clone()).unwrap();
    feature
}

fn count_mr_merged_rows(db: &SqliteAdapter) -> i64 {
    let conn = db.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM notifications WHERE kind = 'mr_merged'",
        [],
        |r| r.get(0),
    )
    .unwrap()
}

// ── Tests ─────────────────────────────────────────────────────────────────

/// The happy path. A single `check_mr_states` tick on a feature that
/// transitions from `open` to `merged` must insert exactly **one**
/// `MrMerged` notification row and emit exactly **one** `MrMerged`
/// domain event. The desktop bell (`NotificationBell`) and the OS
/// notification both depend on this 1:1 invariant — a stray second row
/// shows up as a duplicate "MR for X was merged" in the bell, which is
/// the bug this file reproduces.
#[test]
fn check_mr_states_records_merged_transition_once() {
    let db = open_db();
    let feature = seed_project_and_open_mr(&db, "f-once", "https://example/mr/1");
    let publisher = Arc::new(StubPublisherAlwaysMerged::default());
    let notif = Arc::new(CapturingNotif::default());

    // Run the monitor's per-tick helper directly. Private to
    // `mr_monitor`, but accessible from this `#[path = "..."]`
    // included test submodule.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _ = rt.block_on(super::check_mr_states(&db, &*publisher, &db, &*notif));

    // Exactly one notification row, exactly one live event.
    assert_eq!(
        count_mr_merged_rows(&db),
        1,
        "merged transition must insert exactly one MrMerged row"
    );
    let events = notif.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, DomainEvent::MrMerged { .. }))
            .count(),
        1,
        "merged transition must emit exactly one MrMerged event"
    );

    // The feature's mr_state must have advanced to "merged" so the
    // next tick of `list_with_open_mr` skips it.
    let refreshed = FeatureRepository::get(&db, &feature.id)
        .unwrap()
        .expect("feature still present");
    assert_eq!(refreshed.mr_state.as_deref(), Some("merged"));
}

/// Bug reproduction — the user's symptom. The bell shows two "MR for
/// X was merged" rows for the **same** MR. The monitor's normal
/// `list_with_open_mr` filter (`mr_state = 'open'`) usually keeps a
/// re-tick from re-recording the same feature, but
/// `record_merged` itself has **no idempotency guard**:
///
///   1. `record_merged` inserts a `MrMerged` row unconditionally.
///   2. `NotificationRepository::add` has no `(feature_id, kind)`
///      uniqueness — `id` is a wall-clock-derived primary key
///      (`format!("notif-{}", now_ms())`), so any pair of calls
///      separated by ≥1 ms lands two distinct `id`s and **both
///      rows land in the table**.
///   3. `notif.emit(&DomainEvent::MrMerged { .. })` fires every
///      time — so the React bell gets two `mr_merged` events, each
///      of which refreshes its items list and toasts.
///
/// The realistic trigger is a brief race that re-runs the merged
/// path for the same feature — e.g. an app restart between the
/// `feature.update` and the `notifications.add` (one INSERT
/// succeeded, the second tick re-fires `record_merged`), a
/// future feature-clone path that re-uses an `mr_url`, or just a
/// duplicate tick. With the current code, any of these produce a
/// duplicate row + duplicate event.
///
/// This test reproduces the **row** symptom by spacing the two
/// calls by enough wall-clock to land distinct `id`s, then
/// asserts the post-fix contract. We don't depend on real time —
/// we seed the first row with a known id, then sleep just enough
/// to bump `now_ms()`, then invoke the second call.
#[test]
fn record_merged_twice_for_same_feature_is_not_idempotent() {
    let db = open_db();
    let feature = seed_project_and_open_mr(&db, "f-twice", "https://example/mr/2");
    let notif = Arc::new(CapturingNotif::default());

    // First call — succeeds. Feature row → `mr_state = 'merged'`.
    super::record_merged(&feature, "merged", &db, &db, &*notif).unwrap();

    // Even after `record_merged` flipped `mr_state` to `'merged'`,
    // a second call (simulated — from a duplicate tick or any
    // other re-entry path) must still produce the same single
    // notification row + single event.
    //
    // Bump `now_ms()` past 1 ms so the wall-clock-derived `id`
    // differs; without this padding the second call would simply
    // hit the PRIMARY KEY collision and bail with a different
    // failure mode, not the "two rows" path the user reports.
    std::thread::sleep(std::time::Duration::from_millis(3));
    super::record_merged(&feature, "merged", &db, &db, &*notif).unwrap();

    // Post-fix contract: a duplicate `record_merged` is a no-op,
    // not a double-recording. Exactly one row, exactly one event.
    assert_eq!(
        count_mr_merged_rows(&db),
        1,
        "duplicate record_merged calls must not produce duplicate MrMerged rows"
    );
    let events = notif.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, DomainEvent::MrMerged { .. }))
            .count(),
        1,
        "duplicate record_merged calls must not emit duplicate MrMerged events"
    );
}

/// Regression guard for the *monitor path* (not the bare
/// `record_merged` path). The full `check_mr_states` loop today
/// happens to be idempotent because `list_with_open_mr` filters by
/// `mr_state = 'open'`, so the loop skips a feature after the first
/// `record_merged` updates its row. Pin that behavior so a future
/// change to the SQL filter or to `record_merged` doesn't quietly
/// start inserting duplicates again. The first tick produces the row;
/// the second tick must produce no new row, no new event, and must
/// NOT call the publisher a second time (cheap to detect — publisher
/// call count stays at 1).
#[test]
fn check_mr_states_two_ticks_against_same_feature_is_idempotent() {
    let db = open_db();
    let _ = seed_project_and_open_mr(&db, "f-tick2", "https://example/mr/3");
    let publisher = Arc::new(StubPublisherAlwaysMerged::default());
    let notif = Arc::new(CapturingNotif::default());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(super::check_mr_states(&db, &*publisher, &db, &*notif))
        .unwrap();
    rt.block_on(super::check_mr_states(&db, &*publisher, &db, &*notif))
        .unwrap();

    assert_eq!(
        count_mr_merged_rows(&db),
        1,
        "second tick must not re-insert a MrMerged row"
    );
    let events = notif.events.lock().unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, DomainEvent::MrMerged { .. }))
            .count(),
        1,
        "second tick must not re-emit MrMerged"
    );
    assert_eq!(
        publisher.calls.lock().unwrap().len(),
        1,
        "second tick must skip the already-merged feature without hitting the publisher"
    );
}
