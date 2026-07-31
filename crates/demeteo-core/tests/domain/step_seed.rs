//! What a run's step rows look like the instant they are registered.
//!
//! Pure over a step list: no port double, no async runtime, no driver.

use super::*;
use crate::domain::ids::StepId;

fn steps() -> Vec<StepConfig> {
    vec![
        StepConfig {
            id: StepId::from("s-research".to_string()),
            kind: "agent".to_string(),
            ..Default::default()
        },
        StepConfig {
            id: StepId::from("s-gate".to_string()),
            kind: "gate".to_string(),
            ..Default::default()
        },
    ]
}

#[test]
fn ids_are_derived_from_the_feature_and_step_pair() {
    let rows = seed_step_executions(&FeatureId::from("f-42".to_string()), &steps(), 0);

    assert_eq!(
        rows.iter().map(|r| r.id.0.as_str()).collect::<Vec<_>>(),
        vec!["se-f-42-s-research", "se-f-42-s-gate"],
    );
}

#[test]
fn seeding_the_same_feature_twice_names_the_same_rows() {
    let f = FeatureId::from("f-42".to_string());

    let first = seed_step_executions(&f, &steps(), 1_000);
    let second = seed_step_executions(&f, &steps(), 2_000);

    assert_eq!(
        first.iter().map(|r| r.id.0.clone()).collect::<Vec<_>>(),
        second.iter().map(|r| r.id.0.clone()).collect::<Vec<_>>(),
    );
}

#[test]
fn step_index_follows_the_configured_order() {
    let rows = seed_step_executions(&FeatureId::from("f-42".to_string()), &steps(), 0);

    assert_eq!(
        rows.iter().map(|r| r.step_index).collect::<Vec<_>>(),
        vec![0, 1],
    );
    assert_eq!(
        rows.iter()
            .map(|r| r.step_kind.as_str())
            .collect::<Vec<_>>(),
        vec!["agent", "gate"],
    );
}

#[test]
fn a_seeded_row_is_pending_with_zeroed_spend() {
    let rows = seed_step_executions(&FeatureId::from("f-42".to_string()), &steps(), 7);
    let first = rows.first().expect("two steps in, two rows out");

    assert_eq!(first.status, "pending");
    assert_eq!(first.cost_usd, Some(0.0));
    assert_eq!(first.tokens, Some(0));
    assert_eq!(first.wall_clock_secs, Some(0));
    assert_eq!(first.iteration_count, 0);
    assert!(first.error_message.is_none());
    assert!(first.artifact_paths.is_empty());
    assert_eq!(first.created_at, 7);
    assert_eq!(first.updated_at, 7);
}
