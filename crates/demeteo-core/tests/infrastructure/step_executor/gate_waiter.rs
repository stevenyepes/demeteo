//! The teardown sweep is scoped to the feature tearing down.
//!
//! One run finishing used to `clear()` the shared map, so a *second* run
//! parked at a gate lost its rendezvous while still holding a live driver:
//! every later approve was written to SQLite, emitted as an event, and
//! delivered to nothing, and only restarting the app moved the run again.
//! Both assertions below are about the survivor, not the sweeper.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::adapters::step_executor::gate_waiter::{sweep_feature, GateWaiter};
use crate::domain::ids::FeatureId;
use crate::domain::models::GateDecision;
use crate::paths;

fn waiters(keys: &[&str]) -> Mutex<HashMap<String, std::sync::Arc<GateWaiter>>> {
    Mutex::new(
        keys.iter()
            .map(|k| ((*k).to_string(), GateWaiter::new()))
            .collect(),
    )
}

fn decision(step_execution_id: &str) -> GateDecision {
    GateDecision {
        id: crate::domain::ids::GateDecisionId::from(format!("gd-{step_execution_id}")),
        step_execution_id: crate::domain::ids::StepExecutionId::from(step_execution_id.to_string()),
        decision: Some("approve".to_string()),
        feedback: None,
        created_at: paths::now_ms(),
    }
}

#[test]
fn leaves_another_features_waiter_registered() {
    let map = waiters(&[
        "se-f-1786463096496-s-gate-ship",
        "se-f-1786464484350-s-gate-ship",
    ]);

    sweep_feature(&map, &FeatureId::from("f-1786464484350".to_string()));

    let remaining: Vec<String> = map.lock().unwrap().keys().cloned().collect();
    assert_eq!(
        remaining,
        vec!["se-f-1786463096496-s-gate-ship".to_string()]
    );
}

#[tokio::test]
async fn the_survivor_still_receives_its_decision() {
    let map = waiters(&["se-f-1786463096496-s-gate-ship"]);

    sweep_feature(&map, &FeatureId::from("f-1786464484350".to_string()));

    // What `gate_decide`'s fast path does: look the waiter up by
    // step-execution id and hand it the row it just wrote.
    let waiter = map
        .lock()
        .unwrap()
        .get("se-f-1786463096496-s-gate-ship")
        .cloned()
        .expect("the parked run's waiter is not the finishing run's to drop");
    waiter.deliver(decision("se-f-1786463096496-s-gate-ship"));

    assert_eq!(
        waiter.wait().await.and_then(|d| d.decision),
        Some("approve".to_string())
    );
}

#[test]
fn drops_every_step_of_the_feature_that_finished() {
    let map = waiters(&[
        "se-f-1786464484350-s-gate-review",
        "se-f-1786464484350-s-gate-ship",
        "se-f-1786463096496-s-gate-ship",
    ]);

    sweep_feature(&map, &FeatureId::from("f-1786464484350".to_string()));

    assert_eq!(map.lock().unwrap().len(), 1);
}
