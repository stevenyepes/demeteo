// Tests extracted from `crates/demeteo-core/src/application/discovery/running.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn id() -> DiscoveryId {
    DiscoveryId::from("d-1".to_string())
}

#[test]
fn a_discovery_runs_a_turn_only_while_the_claim_is_held() {
    let turns = RunningTurns::default();
    assert!(!turns.running(&id()));
    {
        let _claim = turns.claim("d-1");
        assert!(turns.running(&id()));
    }
    assert!(!turns.running(&id()));
}

/// The count is the whole reason this is not a set: the first of two
/// overlapping turns to finish must not report the second as over.
#[test]
fn the_first_of_two_turns_to_finish_does_not_end_the_other() {
    let turns = RunningTurns::default();
    let first = turns.claim("d-1");
    let _second = turns.claim("d-1");
    drop(first);
    assert!(turns.running(&id()));
}

#[test]
fn one_discovery_running_says_nothing_about_another() {
    let turns = RunningTurns::default();
    let _claim = turns.claim("d-1");
    assert!(!turns.running(&DiscoveryId::from("d-2".to_string())));
}
