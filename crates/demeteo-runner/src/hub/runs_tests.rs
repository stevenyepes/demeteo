// Tests extracted from `crates/demeteo-runner/src/hub/runs.rs` (mirrored-tests convention). `super` = that module.

use super::RunnerHubRuns;
use crate::test_services::Rig;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::ports::hub::{HubRunControl, StartSupport};

#[tokio::test]
async fn the_hub_cancels_a_run_whichever_desktop_submitted_it() {
    let rig = Rig::new("runner_hub_runs_any_client");
    rig.seed_run("run-1", "f-1", "laptop-1", "running");
    rig.seed_run("run-2", "f-2", "laptop-2", "running");
    let runs = RunnerHubRuns::new(rig.svc.clone());

    runs.cancel(&FeatureId::from("f-2"))
        .await
        .expect("cancelled");

    assert_eq!(rig.run("run-2").status, "cancelled");
    assert_eq!(rig.event_kinds("run-2"), ["cancelled"]);
    assert_eq!(rig.svc.creds.get("run-2"), None);
    assert_eq!(rig.run("run-1").status, "running", "only the run it named");
    assert_eq!(rig.executor.cancels(), ["f-2"]);
}

#[tokio::test]
async fn a_feature_with_no_run_here_is_not_found_and_nothing_is_stopped() {
    let rig = Rig::new("runner_hub_runs_absent");
    rig.seed_run("run-1", "f-1", "laptop-1", "running");
    let runs = RunnerHubRuns::new(rig.svc.clone());

    let error = runs
        .cancel(&FeatureId::from("run-1"))
        .await
        .expect_err("a run id names no feature");

    assert!(error.to_string().contains("run-1"), "{error}");
    assert!(rig.executor.cancels().is_empty());
    assert_eq!(rig.run("run-1").status, "running");
}

#[tokio::test]
async fn a_runner_refuses_hub_starts() {
    let rig = Rig::new("runner_hub_runs_starts");
    assert!(matches!(
        RunnerHubRuns::new(rig.svc.clone()).starts(),
        StartSupport::Refused
    ));
}
