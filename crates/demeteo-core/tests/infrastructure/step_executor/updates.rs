// Tests for `src/adapters/step_executor/updates.rs` (mirrored-tests
// convention). `super` resolves to that module.
//
// The property under test is the one the project list depends on: while a
// human is being waited on, `features.status` says so. `FeatureDetail` derives
// its headline from the steps and would read "gate needs you" whether or not
// the feature row moved; the "Needs you" band and the rail badge read the row
// alone, and a park that only touched the step left a parked run in "Active".

use super::*;
use crate::adapters::database::SqliteAdapter;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::ProjectId;
use crate::domain::models::{Feature, Project};
use crate::ports::db::ProjectRepository;
use rusqlite::Connection;
use std::sync::Mutex;

/// Records every `FeatureStatusChanged`; anything else is a test bug, not a
/// default to swallow.
#[derive(Default)]
struct NotifDouble {
    statuses: Mutex<Vec<String>>,
}

impl NotificationPort for NotifDouble {
    fn emit(&self, event: &DomainEvent) -> Result<(), String> {
        match event {
            DomainEvent::FeatureStatusChanged { status, .. } => self
                .statuses
                .lock()
                .expect("not poisoned")
                .push(status.clone()),
            other => panic!("a feature park emits FeatureStatusChanged, not {other:?}"),
        }
        Ok(())
    }
}

fn feature_at(status: &str) -> (SqliteAdapter, FeatureId) {
    let adapter = SqliteAdapter::new(Connection::open_in_memory().unwrap()).unwrap();
    ProjectRepository::add(
        &adapter,
        Project {
            id: ProjectId::from("p-park".to_string()),
            name: "park".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: 1_700_000_000,
        },
    )
    .unwrap();
    let f_id = FeatureId::from("f-park".to_string());
    FeatureRepository::add(
        &adapter,
        Feature {
            id: f_id.clone(),
            project_id: ProjectId::from("p-park".to_string()),
            workflow_id: None,
            workflow_version_id: None,
            title: "park".to_string(),
            description: String::new(),
            status: status.to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: 1_700_000_000,
            agent_kind: None,
            model: None,
            effort: None,
            mr_url: None,
            mr_state: Some("none".to_string()),
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
        },
    )
    .unwrap();
    (adapter, f_id)
}

fn status_of(adapter: &SqliteAdapter, f_id: &FeatureId) -> String {
    FeatureRepository::get(adapter, f_id)
        .unwrap()
        .expect("feature row")
        .status
}

#[test]
fn park_moves_a_running_feature_to_awaiting_gate_and_announces_it() {
    let (adapter, f_id) = feature_at("running");
    let notif = NotifDouble::default();

    park_feature(&adapter, &notif, &f_id);

    assert_eq!(status_of(&adapter, &f_id), "awaiting_gate");
    assert_eq!(
        *notif.statuses.lock().unwrap(),
        vec!["awaiting_gate".to_string()],
        "the list patches its row off this event; a silent write leaves it stale until reload"
    );
}

#[test]
fn park_never_touches_a_feature_that_is_not_running() {
    for status in [
        "completed",
        "failed",
        "cancelled",
        "awaiting_mr",
        "awaiting_gate",
    ] {
        let (adapter, f_id) = feature_at(status);
        let notif = NotifDouble::default();

        park_feature(&adapter, &notif, &f_id);

        assert_eq!(status_of(&adapter, &f_id), status, "{status}");
        assert!(
            notif.statuses.lock().unwrap().is_empty(),
            "{status}: nothing to announce"
        );
    }
}

#[test]
fn ensure_running_unparks_only_the_transient_statuses() {
    for status in ["awaiting_gate", "gated", "bootstrapping"] {
        let (adapter, f_id) = feature_at(status);
        let notif = NotifDouble::default();

        ensure_feature_running(&adapter, &notif, &f_id);

        assert_eq!(status_of(&adapter, &f_id), "running", "{status}");
        assert_eq!(
            *notif.statuses.lock().unwrap(),
            vec!["running".to_string()],
            "{status}"
        );
    }
    for status in ["running", "completed", "failed", "cancelled"] {
        let (adapter, f_id) = feature_at(status);
        let notif = NotifDouble::default();

        ensure_feature_running(&adapter, &notif, &f_id);

        assert_eq!(status_of(&adapter, &f_id), status, "{status}");
        assert!(notif.statuses.lock().unwrap().is_empty(), "{status}");
    }
}

#[test]
fn a_park_and_a_resume_round_trip() {
    let (adapter, f_id) = feature_at("running");
    let notif = NotifDouble::default();

    park_feature(&adapter, &notif, &f_id);
    ensure_feature_running(&adapter, &notif, &f_id);

    assert_eq!(status_of(&adapter, &f_id), "running");
    assert_eq!(
        *notif.statuses.lock().unwrap(),
        vec!["awaiting_gate".to_string(), "running".to_string()]
    );
}
