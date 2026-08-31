// Tests for `crates/demeteo-core/src/application/ask/running.rs`, moved here per the
// mirrored-tests convention. `super` = that module.

use super::*;

fn id() -> AskThreadId {
    AskThreadId::from("t-1".to_string())
}

#[test]
fn an_ask_thread_runs_a_turn_only_while_the_claim_is_held() {
    let turns = Arc::new(RunningTurns::default());
    assert!(!turns.running(&id()));
    {
        let _claim = turns.clone().try_claim("t-1");
        assert!(turns.running(&id()));
    }
    assert!(!turns.running(&id()));
}

/// A second overlapping claim on one thread must not make the first's drop
/// end the turn early.
#[test]
fn the_first_of_two_turns_to_finish_does_not_end_the_other() {
    let turns = Arc::new(RunningTurns::default());
    let (first, _) = turns.clone().claim("t-1");
    let (_second, already_running) = turns.clone().claim("t-1");
    assert_eq!(already_running, 1);
    drop(first);
    assert!(turns.running(&id()));
}

#[test]
fn a_refused_turn_leaves_the_running_one_holding_its_claim() {
    let turns = Arc::new(RunningTurns::default());
    let running = turns
        .clone()
        .try_claim("t-1")
        .expect("the first turn claims it");
    assert!(turns.clone().try_claim("t-1").is_none());
    assert!(
        turns.running(&id()),
        "the refusal must not release the claim"
    );
    drop(running);
    assert!(!turns.running(&id()));
    assert!(
        turns.clone().try_claim("t-1").is_some(),
        "and the next turn may have it"
    );
}

#[test]
fn one_ask_thread_running_says_nothing_about_another() {
    let turns = Arc::new(RunningTurns::default());
    let _claim = turns.clone().try_claim("t-1");
    assert!(!turns.running(&AskThreadId::from("t-2".to_string())));
}
