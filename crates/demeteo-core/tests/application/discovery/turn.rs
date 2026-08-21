// Tests extracted from `crates/demeteo-core/src/application/discovery/turn.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn a_resumed_turn_that_said_nothing_and_failed_is_retried_from_the_transcript() {
    assert!(should_reseed_and_retry(true, false, TurnEnding::Failed));
    assert!(should_reseed_and_retry(
        true,
        false,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_resumed_turn_that_answered_before_failing_reached_the_model() {
    assert!(!should_reseed_and_retry(true, true, TurnEnding::Failed));
    assert!(!should_reseed_and_retry(
        true,
        true,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_turn_that_already_carried_the_transcript_has_nothing_to_fall_back_to() {
    assert!(!should_reseed_and_retry(false, false, TurnEnding::Failed));
    assert!(!should_reseed_and_retry(
        false,
        false,
        TurnEnding::Environmental
    ));
}

#[test]
fn a_stop_is_the_user_declining_the_turn_not_a_lost_session() {
    assert!(!should_reseed_and_retry(
        true,
        false,
        TurnEnding::Interrupted
    ));
}

#[test]
fn a_turn_that_worked_is_never_run_twice() {
    assert!(!should_reseed_and_retry(true, false, TurnEnding::Success));
}

#[test]
fn every_ending_but_a_stop_reports_what_it_spent() {
    let outcome = || TurnOutcome {
        text: "said something".to_string(),
        produced_artifacts: Vec::new(),
        cost_usd: 0.31,
        tokens: 12_400,
        cache_read_input_tokens: 0,
        cache_creation_input_tokens: 0,
    };
    for result in [
        TurnResult::Success(outcome()),
        TurnResult::Failed {
            reason: "rate limited".to_string(),
            spent: outcome(),
        },
        TurnResult::Environmental {
            reason: "wall cap".to_string(),
            spent: outcome(),
        },
    ] {
        let (_, _, spent) = split(result);
        assert_eq!(spent.cost_usd, 0.31);
        assert_eq!(spent.tokens, 12_400);
    }
    let (ending, reason, spent) = split(TurnResult::Interrupted);
    assert_eq!(ending, TurnEnding::Interrupted);
    assert_eq!(reason, None);
    assert_eq!(spent.cost_usd, 0.0);
}

#[test]
fn the_interviewer_may_read_and_run_but_never_write() {
    let p = interviewer_permissions();
    assert_eq!(p.read_fs, Access::Allow);
    assert_eq!(p.write_fs, Access::Deny);
    assert_eq!(p.execute, Access::Allow);
    assert_eq!(p.network, Access::Allow);
}
