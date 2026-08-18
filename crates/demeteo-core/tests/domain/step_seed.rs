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

#[test]
fn a_seeded_id_belongs_to_the_feature_that_seeded_it() {
    let f = FeatureId::from("f-42".to_string());

    for row in seed_step_executions(&f, &steps(), 0) {
        assert!(belongs_to_feature(&f, row.id.0.as_str()));
    }
}

#[test]
fn another_features_rows_do_not_belong_to_it() {
    let mine = FeatureId::from("f-42".to_string());
    let theirs = FeatureId::from("f-43".to_string());

    for row in seed_step_executions(&theirs, &steps(), 0) {
        assert!(!belongs_to_feature(&mine, row.id.0.as_str()));
    }
}

/// The separator is load-bearing: without it every `f-4*` row would answer to
/// `f-4`, and a teardown would reach into runs it has no relationship to.
#[test]
fn a_feature_id_that_prefixes_another_does_not_claim_its_rows() {
    let short = FeatureId::from("f-4".to_string());

    for row in seed_step_executions(&FeatureId::from("f-42".to_string()), &steps(), 0) {
        assert!(!belongs_to_feature(&short, row.id.0.as_str()));
    }
}

// ── The row a sync outside the run reports through ───────────────────────────

/// It has to be findable, because `step_create` is a bare `INSERT`: the second
/// manual sync on a feature looks the first one's row up rather than colliding
/// with it, and that only works while the id is derived from the pair.
#[test]
fn the_manual_sync_row_is_named_from_the_feature_not_minted() {
    let f = FeatureId::from("f-42".to_string());

    let first = manual_sync_step_execution(&f, 100);
    let second = manual_sync_step_execution(&f, 200);

    assert_eq!(first.id, second.id);
    assert!(belongs_to_feature(&f, first.id.0.as_str()));
    assert_eq!(first.step_id.0, MANUAL_SYNC_STEP_ID);
}

/// Both guards that fall back to index order when the graph will not resolve —
/// `active_predecessor_refusal` and the replay rewind — read a lower index as
/// upstream. A manual sync sitting at index 0 would be a "predecessor" that
/// blocks every retry and every gate decision on the feature.
#[test]
fn the_manual_sync_row_sorts_after_every_graph_node() {
    let row = manual_sync_step_execution(&FeatureId::from("f-42".to_string()), 0);

    assert_eq!(row.step_index, u32::MAX);
    for graph_row in seed_step_executions(&FeatureId::from("f-42".to_string()), &steps(), 0) {
        assert!(graph_row.step_index < row.step_index);
    }
}
