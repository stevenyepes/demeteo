// Tests extracted from `crates/demeteo-core/src/application/turn_retry.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::application::discovery::events::TurnEnding;

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
