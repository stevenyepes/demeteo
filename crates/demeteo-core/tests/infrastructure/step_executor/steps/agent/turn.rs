// Tests for the agent turn's spend fold. `super` = the `turn` module.
//
// No doubles and no runtime: `apply_turn_result` is the half of the turn
// that decides, and it was unreachable from a test while it lived inside a
// 1030-line `async fn` that also spawned an agent.

use super::*;

use crate::adapters::agent::event_stream::TurnOutcome;
use crate::domain::artifact::Artifact;

/// A spend snapshot with both cache slots already carrying a previous
/// turn's numbers, so a fold that fails to write is distinguishable from
/// one that writes `None`.
struct Slots {
    cost: f64,
    tokens: i64,
    cache_read: Option<u64>,
    cache_creation: Option<u64>,
}

impl Slots {
    fn seeded() -> Self {
        Self {
            cost: 1.5,
            tokens: 100,
            cache_read: Some(7),
            cache_creation: Some(9),
        }
    }

    fn spend(&mut self) -> AgentSpend<'_> {
        AgentSpend {
            cost: &mut self.cost,
            tokens: &mut self.tokens,
            start: std::time::Instant::now(),
            cache_read: &mut self.cache_read,
            cache_creation: &mut self.cache_creation,
        }
    }
}

fn outcome(cost: f64, tokens: i64, cache_read: u64, cache_creation: u64) -> TurnOutcome {
    TurnOutcome {
        text: "the reply".into(),
        produced_artifacts: vec![Artifact::tool_write(
            "report",
            "artifacts/report.md",
            "body",
        )],
        cost_usd: cost,
        tokens,
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: cache_creation,
    }
}

fn success(cost: f64, tokens: i64, cache_read: u64, cache_creation: u64) -> TurnResult {
    TurnResult::Success(outcome(cost, tokens, cache_read, cache_creation))
}

#[test]
fn a_successful_turn_accumulates_cost_and_tokens() {
    let mut slots = Slots::seeded();
    let mut spend = slots.spend();
    let disposition = apply_turn_result(success(0.25, 40, 11, 13), &mut spend);

    assert!(matches!(disposition, TurnDisposition::Answered { .. }));
    assert_eq!(slots.cost, 1.75, "cost accumulates, it does not replace");
    assert_eq!(slots.tokens, 140, "tokens accumulate too");
}

#[test]
fn a_successful_turn_replaces_both_cache_slots() {
    let mut slots = Slots::seeded();
    let mut spend = slots.spend();
    let _ = apply_turn_result(success(0.0, 0, 11, 13), &mut spend);

    assert_eq!(
        (slots.cache_read, slots.cache_creation),
        (Some(11), Some(13)),
        "the cache chip shows the latest turn's counts, not the sum"
    );
}

#[test]
fn a_successful_turn_hands_back_its_text_and_artifacts() {
    let mut slots = Slots::seeded();
    let mut spend = slots.spend();
    match apply_turn_result(success(0.0, 0, 0, 0), &mut spend) {
        TurnDisposition::Answered { text, produced } => {
            assert_eq!(text, "the reply");
            assert_eq!(produced.len(), 1);
            assert_eq!(produced[0].name, "report");
        }
        _ => panic!("a Success must be Answered"),
    }
}

#[test]
fn an_interrupted_turn_spends_nothing_and_touches_no_slot() {
    let mut slots = Slots::seeded();
    let mut spend = slots.spend();
    let disposition = apply_turn_result(TurnResult::Interrupted, &mut spend);

    assert!(matches!(disposition, TurnDisposition::Cancelled));
    assert_eq!(slots.cost, 1.5);
    assert_eq!(slots.tokens, 100);
    assert_eq!(
        (slots.cache_read, slots.cache_creation),
        (Some(7), Some(9)),
        "an interrupted turn must not blank the previous turn's counts"
    );
}

/// A turn that failed still bought what it read.
///
/// The tokens are gone whether or not the turn reached a verdict — a tripped
/// `--max-turns`, a dollar ceiling, an API error mid-flight — and a step whose
/// budget and retry ladder are counted in dollars has to see them. Left
/// unbilled, the most expensive turns in a run were the ones that reported
/// costing nothing.
#[test]
fn an_agent_failure_is_failed_and_bills_what_it_spent() {
    let mut slots = Slots::seeded();
    let mut spend = slots.spend();
    let disposition = apply_turn_result(
        TurnResult::Failed {
            reason: "model refused".into(),
            spent: outcome(0.25, 40, 11, 13),
        },
        &mut spend,
    );

    match disposition {
        TurnDisposition::Broken(StepOutcome::Failed(msg)) => assert_eq!(msg, "model refused"),
        _ => panic!("an agent failure must be Broken(Failed)"),
    }
    assert_eq!(slots.cost, 1.75, "the failed turn's dollars are not free");
    assert_eq!(slots.tokens, 140);
    assert_eq!(
        (slots.cache_read, slots.cache_creation),
        (Some(11), Some(13))
    );
}

#[test]
fn a_broken_box_is_environmental_not_failed() {
    let mut slots = Slots::seeded();
    let mut spend = slots.spend();
    let disposition = apply_turn_result(
        TurnResult::Environmental {
            reason: "silence timeout".into(),
            spent: outcome(0.25, 40, 11, 13),
        },
        &mut spend,
    );

    match disposition {
        TurnDisposition::Broken(StepOutcome::Environmental(msg)) => {
            assert_eq!(msg, "silence timeout");
        }
        _ => panic!(
            "an environmental failure must not become Failed — \
             that would route a broken box into the re-implementation retry loop"
        ),
    }
    assert_eq!(slots.cost, 1.75, "a turn the box broke still bought tokens");
}
