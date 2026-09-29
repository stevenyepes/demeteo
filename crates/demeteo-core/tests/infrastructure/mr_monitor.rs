use super::*;

use std::sync::atomic::{AtomicUsize, Ordering};

use rusqlite::Connection;

use crate::adapters::database::SqliteAdapter;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::ProjectId;
use crate::domain::models::{Feature, Project};
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::worktree_ops::FeatureCachePort;

/// Counts how many `MrMerged` live events were emitted, so a test can
/// assert the bell/toast fires exactly once even if `record_merged`
/// runs more than once.
#[derive(Default)]
struct CountingNotif {
    mr_merged: AtomicUsize,
}

impl NotificationPort for CountingNotif {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        if matches!(event, DomainEvent::MrMerged { .. }) {
            self.mr_merged.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}

fn setup() -> SqliteAdapter {
    let conn = Connection::open_in_memory().unwrap();
    SqliteAdapter::new(conn).unwrap()
}

/// Insert a project + a feature with an open MR, and return the row so
/// it can be handed to `record_merged` (which takes `&Feature`).
fn make_open_mr_feature(adapter: &SqliteAdapter, id: &str, project_id: &str) -> Feature {
    let pid = ProjectId::from(project_id.to_string());
    // Tolerant insert: a test may reuse a project across features.
    let _ = ProjectRepository::add(
        adapter,
        Project {
            id: pid.clone(),
            name: format!("project_{}", project_id),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 1,
            spend: 0.0,
            tokens: 0,
            created_at: 1000,
        },
    );

    let feature = Feature {
        effort: None,
        id: FeatureId::from(id.to_string()),
        project_id: pid,
        workflow_id: None,
        workflow_version_id: None,
        title: "Add the widget".to_string(),
        description: String::new(),
        status: "running".to_string(),
        total_cost: 0.0,
        tokens: 0,
        duration: "0s".to_string(),
        created_at: 1000,
        agent_kind: None,
        model: None,
        mr_url: Some("https://github.com/o/r/pull/1".to_string()),
        mr_state: Some("open".to_string()),
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
    FeatureRepository::add(adapter, feature.clone()).unwrap();
    feature
}

fn mr_merged_count(adapter: &SqliteAdapter, project_id: &str) -> usize {
    let pid = ProjectId::from(project_id.to_string());
    NotificationRepository::list(adapter, Some(&pid), u32::MAX)
        .unwrap()
        .iter()
        .filter(|n| n.kind == NotificationKind::MrMerged)
        .count()
}

/// The happy path: a first merge persists exactly one notification,
/// emits one live event, and drives the feature row to its terminal
/// `completed` / `merged` state.
#[test]
fn record_merged_completes_feature_and_notifies_once() {
    let adapter = setup();
    let feature = make_open_mr_feature(&adapter, "f-1", "p-1");
    let notif = CountingNotif::default();

    record_merged(&feature, "merged", &adapter, &adapter, &notif).unwrap();

    assert_eq!(mr_merged_count(&adapter, "p-1"), 1);
    assert_eq!(notif.mr_merged.load(Ordering::SeqCst), 1);

    let stored = FeatureRepository::get(&adapter, &feature.id)
        .unwrap()
        .expect("feature row should exist");
    assert_eq!(stored.status, "completed");
    assert_eq!(stored.mr_state.as_deref(), Some("merged"));
}

/// Regression for the duplicate-notification bug: if `record_merged`
/// runs again for a feature that already has an `MrMerged` notification
/// (e.g. the MR row got re-polled after its `mr_state` was reset), the
/// guard must short-circuit — no second notification row, no second
/// live event.
#[test]
fn record_merged_is_idempotent_for_the_same_feature() {
    let adapter = setup();
    let feature = make_open_mr_feature(&adapter, "f-1", "p-1");
    let notif = CountingNotif::default();

    record_merged(&feature, "merged", &adapter, &adapter, &notif).unwrap();
    // Second call stands in for a re-poll of the same merged MR.
    record_merged(&feature, "merged", &adapter, &adapter, &notif).unwrap();

    assert_eq!(
        mr_merged_count(&adapter, "p-1"),
        1,
        "a repeated merge must not persist a duplicate notification"
    );
    assert_eq!(
        notif.mr_merged.load(Ordering::SeqCst),
        1,
        "a repeated merge must not re-emit the live bell/toast event"
    );
}

/// Answers `fetch_mr_state` with one fixed state; any other call is a test bug.
struct FixedMrState(&'static str);

#[async_trait::async_trait]
impl MrPublisher for FixedMrState {
    async fn publish_mr(
        &self,
        _: &str,
        _: &FeatureId,
        _: crate::domain::models::PublishOptions,
    ) -> Result<crate::domain::models::MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_state(&self, _: &str, _: &str) -> Result<String, String> {
        Ok(self.0.to_string())
    }
    async fn list_open_mrs(
        &self,
        _: &str,
        _: Option<&str>,
    ) -> Result<Vec<crate::domain::mr_summary::MrSummary>, crate::domain::mr_list_error::MrListError>
    {
        panic!("unexpected MrPublisher call")
    }
    async fn fetch_mr_detail(
        &self,
        _: &str,
        _: &str,
    ) -> Result<crate::domain::mr_summary::MrSummary, crate::domain::mr_list_error::MrListError>
    {
        panic!("unexpected MrPublisher call")
    }
    async fn post_mr_comment(&self, _: &str, _: &str, _: &str) -> Result<String, String> {
        panic!("unexpected MrPublisher call")
    }
    async fn publish_branch_mr(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: crate::domain::models::PublishOptions,
    ) -> Result<crate::domain::models::MrInfo, String> {
        panic!("unexpected MrPublisher call")
    }
}

#[derive(Default)]
struct RecordingCache {
    released: std::sync::Mutex<Vec<String>>,
    mr_states: std::sync::Mutex<Vec<Option<String>>>,
}

#[async_trait::async_trait]
impl FeatureCachePort for RecordingCache {
    async fn release(&self, feature: &Feature) -> Result<(), String> {
        self.released.lock().unwrap().push(feature.id.0.clone());
        self.mr_states
            .lock()
            .unwrap()
            .push(feature.mr_state.clone());
        Err("the monitor must survive a failed release".to_string())
    }
}

/// One poll of a feature in `status` whose PR the provider reports as
/// `provider_state`; returns the features whose cache the poll released.
async fn poll_once(status: &str, provider_state: &'static str) -> Vec<String> {
    poll_once_recording(status, provider_state)
        .await
        .released
        .into_inner()
        .unwrap()
}

async fn poll_once_recording(status: &str, provider_state: &'static str) -> RecordingCache {
    let adapter = Arc::new(setup());
    let feature = make_open_mr_feature(&adapter, "f-1", "p-1");
    FeatureRepository::update(
        &*adapter,
        &feature.id,
        &FeaturePatch {
            status: Some(status.to_string()),
            ..Default::default()
        },
    )
    .unwrap();
    let cache = Arc::new(RecordingCache::default());

    check_mr_states(&MrMonitorPorts {
        features: adapter.clone(),
        mr_publisher: Arc::new(FixedMrState(provider_state)),
        notifications: adapter.clone(),
        notif: Arc::new(CountingNotif::default()),
        tickets: adapter.clone(),
        discoveries: adapter,
        cache: cache.clone(),
    })
    .await
    .unwrap();

    Arc::try_unwrap(cache).unwrap_or_else(|_| panic!("the poll kept the cache port"))
}

/// A runner-owned feature's cache is released by telling its runner *why*, so
/// the port must see the state the poll found, not the `open` it polled from.
#[tokio::test]
async fn the_released_feature_carries_the_polled_pr_state() {
    for state in ["merged", "closed"] {
        let cache = poll_once_recording("completed", state).await;
        assert_eq!(
            cache.mr_states.into_inner().unwrap(),
            [Some(state.to_string())]
        );
    }
}

#[tokio::test]
async fn a_polled_merge_or_close_of_a_finished_feature_releases_its_cache() {
    assert_eq!(poll_once("completed", "merged").await, ["f-1"]);
    assert_eq!(poll_once("completed", "closed").await, ["f-1"]);
}

#[tokio::test]
async fn a_poll_that_leaves_the_feature_runnable_keeps_its_cache() {
    assert!(poll_once("completed", "open").await.is_empty());
    assert!(poll_once("completed", "draft").await.is_empty());
    assert!(poll_once("awaiting_mr", "closed").await.is_empty());
}
