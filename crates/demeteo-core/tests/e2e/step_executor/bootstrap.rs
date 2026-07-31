//! `feature_start`'s eager row and the spawned bootstrap tail behind it.

use std::sync::Arc;

use super::harness::{build_test_executor_with_notif, CapturingNotif};
use crate::domain::ids::{FeatureId, ProjectId};
use crate::paths;
use crate::ports::db::{FeatureRepository, ProjectRepository};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::StepExecutor;

/// `feature_start` inserts the eager row as `bootstrapping` and returns
/// immediately, then runs the bootstrap on a spawned tail that streams
/// `BootstrapProgress` events. A bootstrap that fails (here: a project with no
/// repository, so `resolve_execution_context` bails at the repo check) must
/// emit a `preparing` "failed" phase and drive the feature to `failed` via a
/// `FeatureStatusChanged` — rather than the pre-refactor behavior of returning
/// an error straight from `feature_start`.
#[tokio::test]
async fn test_feature_start_bootstrap_failure_emits_events_and_fails() {
    let notif = Arc::new(CapturingNotif::default());
    let (executor, db, temp_dir) = build_test_executor_with_notif("boot_fail", notif.clone()).await;

    let now = paths::now_ms();
    let projects: &dyn ProjectRepository = &*db;
    projects
        .add(crate::domain::models::Project {
            id: ProjectId::from("p-boot"),
            name: "boot-test".to_string(),
            compute_type: "local".to_string(),
            remote_host: None,
            status: "idle".to_string(),
            nodes: 0,
            spend: 0.0,
            tokens: 0,
            created_at: now,
        })
        .unwrap();
    // Deliberately no repository for the project → the bootstrap tail's
    // `resolve_execution_context` fails at the repo check.

    let feature = executor
        .feature_start(
            None,
            "p-boot",
            "wf-x",
            "Boot Feature",
            "a description",
            None,
            // model / effort / commit_artifacts / loop_iterations / max_budget_usd: all inherit.
            None,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
        )
        .await
        .expect("feature_start returns the eager row, not an error");

    // The returned row is the eager, pre-bootstrap snapshot.
    assert_eq!(
        feature.status, "bootstrapping",
        "feature_start returns immediately with a bootstrapping row"
    );

    // Wait for the spawned tail to reconcile the feature to a terminal state.
    let features: &dyn FeatureRepository = &*db;
    let fid = FeatureId::from(feature.id.0.clone());
    let mut final_status = String::new();
    for _ in 0..200 {
        if let Ok(Some(f)) = features.get(&fid) {
            if f.status == "failed" {
                final_status = f.status;
                break;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert_eq!(
        final_status, "failed",
        "the failed bootstrap tail marks the feature failed"
    );

    let events = notif.events.lock().unwrap();
    let saw_preparing_failed = events.iter().any(|e| {
        matches!(
            e,
            DomainEvent::BootstrapProgress { phase, status, .. }
                if phase == "preparing" && status == "failed"
        )
    });
    assert!(
        saw_preparing_failed,
        "expected a BootstrapProgress{{preparing, failed}} event; got: {:?}",
        *events
    );
    let saw_status_failed = events.iter().any(|e| {
        matches!(
            e,
            DomainEvent::FeatureStatusChanged { status, .. } if status == "failed"
        )
    });
    assert!(
        saw_status_failed,
        "expected a FeatureStatusChanged(failed) event"
    );

    drop(events);
    let _ = std::fs::remove_dir_all(temp_dir);
}
