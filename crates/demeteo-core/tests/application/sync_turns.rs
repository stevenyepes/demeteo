// Tests extracted from `crates/demeteo-core/src/application/sync_turns.rs` (mirrored-tests convention). `super` = that module.

use super::*;

/// The slot has to come back on every way out of a turn, not only the one the
/// author remembered.
///
/// Released by hand, it survived every `?` between the claim and the release —
/// and `feature_resolve_sync_conflicts_impl` has one, three lines after
/// claiming. Nothing sweeps this map, so a leaked entry lasts the life of the
/// process: the feature's next sync and its next resolution are both refused,
/// and since the entry became half of `sync_liveness` every intervention on the
/// session is too. Restarting the app was the only way out.
#[test]
fn a_turn_that_leaves_early_gives_the_slot_back() {
    fn takes_and_fails(turns: &SyncTurns) -> Result<(), String> {
        let _turn = turns.claim("f-1", None).ok_or("taken")?;
        Err("the row this turn wanted could not be created".to_string())?;
        unreachable!()
    }

    let turns = SyncTurns::default();
    assert!(takes_and_fails(&turns).is_err());
    assert!(
        !turns.claimed("f-1"),
        "an early return may not leave the feature claimed forever"
    );
    assert!(
        turns.claim("f-1", None).is_some(),
        "and the next turn has to be able to take it"
    );
}

/// The failure the guard exists for that no `?` can reproduce: the turn's
/// future is dropped mid-flight — a cancelled task, a panicking runtime thread
/// — after the claim and before anything that would have released it.
///
/// Pre-guard this froze the session `Live`: `reconcile` passes a `resolving`
/// row through untouched while something is running it, and every intervention
/// is refused, so a resolution nobody was running could be neither continued
/// nor abandoned.
#[tokio::test]
async fn a_turn_whose_future_is_dropped_gives_the_slot_back() {
    let turns = SyncTurns::default();
    let _ = tokio::time::timeout(std::time::Duration::from_millis(1), async {
        let _turn = turns.claim("f-1", None).expect("the registry is empty");
        std::future::pending::<()>().await;
    })
    .await;

    assert!(
        !turns.claimed("f-1"),
        "a dropped resolution must leave a conflict its user can still abort"
    );
}
