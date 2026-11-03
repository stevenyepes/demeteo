// Tests extracted from `crates/demeteo-runner/src/control.rs` (mirrored-tests convention). `super` = that module.

use super::{cancel_run, run_for_feature};
use crate::test_services::Rig;

#[tokio::test]
async fn a_live_run_is_cancelled_once_with_its_credential_dropped() {
    let rig = Rig::new("runner_control_cancel");
    rig.seed_run("run-1", "f-1", "laptop-1", "running");

    let run = cancel_run(&rig.svc.ctx, &rig.svc.creds, rig.run("run-1"))
        .await
        .expect("cancelled");

    assert_eq!(run.status, "cancelled");
    assert_eq!(rig.run("run-1").status, "cancelled");
    assert_eq!(rig.executor.cancels(), ["f-1"]);
    assert_eq!(rig.event_kinds("run-1"), ["cancelled"]);
    assert_eq!(rig.svc.creds.get("run-1"), None);
}

#[tokio::test]
async fn a_run_that_already_ended_keeps_its_outcome() {
    let rig = Rig::new("runner_control_ended");
    rig.seed_run("run-1", "f-1", "laptop-1", "awaiting_mr");

    let run = cancel_run(&rig.svc.ctx, &rig.svc.creds, rig.run("run-1"))
        .await
        .expect("answered");

    assert_eq!(run.status, "awaiting_mr");
    assert_eq!(rig.run("run-1").status, "awaiting_mr");
    assert!(rig.event_kinds("run-1").is_empty());
    assert!(rig.svc.creds.get("run-1").is_some());
}

#[tokio::test]
async fn a_feature_resolves_to_its_newest_run_and_an_unknown_one_to_none() {
    let rig = Rig::new("runner_control_lookup");
    rig.seed_run("run-old", "f-1", "laptop-1", "failed");
    rig.seed_run("run-other", "f-2", "laptop-1", "running");
    let runs = rig.svc.ctx.runner_runs.as_ref();
    // Created later than every `seed_run` run.
    runs.get_or_create("run-new", "{}", "laptop-2", 5).unwrap();
    runs.update_status(
        "run-new",
        "running",
        Some("p-1"),
        Some("f-1"),
        None,
        None,
        6,
    )
    .unwrap();

    let found = run_for_feature(runs, "f-1").unwrap().expect("a run");
    assert_eq!(found.run_id, "run-new");
    assert!(run_for_feature(runs, "f-9").unwrap().is_none());
    assert!(
        run_for_feature(runs, "run-other").unwrap().is_none(),
        "a run id is not a feature id"
    );
}

// A run whose feature has no driver here — its driver panicked, or a restart
// could not re-arm it — has nothing for `feature_cancel` to signal. The row
// the Hub and the desktop read is the feature's, so it must still end.
#[tokio::test]
async fn a_run_with_no_live_driver_still_ends_its_feature_cancelled() {
    for parked in ["running", "awaiting_gate"] {
        let rig = Rig::with_engine("runner_control_driverless");
        rig.seed_feature("f-1", parked);
        rig.seed_run("run-1", "f-1", "laptop-1", "running");

        let run = cancel_run(&rig.svc.ctx, &rig.svc.creds, rig.run("run-1"))
            .await
            .expect("cancelled");

        assert_eq!(run.status, "cancelled", "{parked}");
        assert_eq!(rig.feature_status("f-1"), "cancelled", "{parked}");
    }
}
