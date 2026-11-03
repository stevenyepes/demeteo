//! What `feature_cancel` leaves on the feature row, with and without a driver
//! to signal.

use std::sync::Arc;

use super::harness::{build_test_executor_in, scratch_dir, CapturingNotif};
use super::sync_base::seed_feature;
use crate::adapters::step_executor::scripted_exec::ScriptedExec;
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::ids::FeatureId;
use crate::ports::db::FeatureRepository;
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::StepExecutor;

fn announced(notif: &CapturingNotif) -> Vec<String> {
    notif
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            DomainEvent::FeatureStatusChanged { status, .. } => Some(status.clone()),
            _ => None,
        })
        .collect()
}

async fn cancel(label: &str, live: bool) -> (String, Vec<String>) {
    let temp_dir = scratch_dir(label);
    let notif = Arc::new(CapturingNotif::default());
    let (executor, db) = build_test_executor_in(
        temp_dir.clone(),
        notif.clone(),
        Arc::new(ScriptedExec::new(&[])),
    )
    .await;
    let feature_id = seed_feature(&db, label, FeatureOrigin::DefaultBranch, None);
    if live {
        executor
            .driver_registry()
            .register(FeatureId::from(feature_id.clone()));
    }

    executor
        .feature_cancel(&feature_id)
        .await
        .expect("cancelled");

    let features: &dyn FeatureRepository = &*db;
    let status = features
        .get(&FeatureId::from(feature_id))
        .unwrap()
        .expect("feature")
        .status;
    let _ = std::fs::remove_dir_all(temp_dir);
    (status, announced(&notif))
}

#[tokio::test]
async fn a_cancel_with_no_driver_ends_the_feature_itself() {
    let (status, announced) = cancel("cancel_driverless", false).await;
    assert_eq!(status, "cancelled");
    assert_eq!(announced, ["cancelled"]);
}

#[tokio::test]
async fn a_cancel_leaves_a_live_drivers_feature_for_the_driver_to_end() {
    let (status, announced) = cancel("cancel_live", true).await;
    assert_eq!(status, "running", "the driver records its own end");
    assert!(announced.is_empty(), "announced twice: {announced:?}");
}
