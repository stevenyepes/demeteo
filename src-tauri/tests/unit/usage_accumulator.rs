//! Pure-logic unit tests for `UsageAccumulator`.
//!
//! No I/O, no agent. Covers the monotonic-max rule, cost precedence,
//! pricing-table fallback, the finished-flag lock, and edge cases
//! (empty stream, out-of-order events, post-terminal events ignored).

use std::collections::HashMap;
use std::sync::Arc;

use crate::domain::agent_event::{AgentEvent, StopReason, Usage};
use crate::domain::usage::UsageAccumulator;
use crate::ports::pricing::{ModelPrice, PricingTable};

/// Tiny in-memory pricing table for tests.
struct TestPricing {
    rows: HashMap<String, ModelPrice>,
}

impl TestPricing {
    fn new() -> Self {
        let mut rows = HashMap::new();
        rows.insert(
            "claude-sonnet-4".to_string(),
            ModelPrice {
                input_per_million: 3.0,
                output_per_million: 15.0,
            },
        );
        rows.insert(
            "free-model".to_string(),
            ModelPrice {
                input_per_million: 0.0,
                output_per_million: 0.0,
            },
        );
        Self { rows }
    }
}

impl PricingTable for TestPricing {
    fn price_for(&self, model: &str) -> Option<ModelPrice> {
        self.rows.get(model).copied()
    }
    fn context_window(&self, _model: &str) -> Option<u64> {
        // Tests don't exercise the watchdog path; return None to
        // match the legacy "no budget data, skip check" behavior.
        None
    }
    fn known_models(&self) -> Vec<String> {
        self.rows.keys().cloned().collect()
    }
}

fn usage(
    input: u64,
    output: u64,
    cost: Option<f64>,
    cache_read: u64,
    cache_create: u64,
) -> AgentEvent {
    AgentEvent::Usage(Usage {
        input_tokens: input,
        output_tokens: output,
        cost_usd: cost,
        cache_read_input_tokens: cache_read,
        cache_creation_input_tokens: cache_create,
    })
}

fn terminal(stop_reason: StopReason, u: Option<Usage>) -> AgentEvent {
    AgentEvent::TurnComplete {
        stop_reason,
        usage: u,
    }
}

#[test]
fn empty_stream_yields_zero() {
    let acc = UsageAccumulator::new(Some("claude-sonnet-4".into()));
    assert_eq!(acc.tokens(), 0);
    assert_eq!(acc.cost_usd(), 0.0);
    assert!(!acc.has_cost());
    assert!(!acc.finished());
}

#[test]
fn single_usage_event_is_recorded() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(100, 50, Some(0.01), 0, 0));
    assert_eq!(acc.tokens(), 150);
    assert_eq!(acc.cost_usd(), 0.01);
    assert!(acc.has_cost());
}

#[test]
fn monotonic_max_on_tokens() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(100, 50, None, 0, 0));
    acc.ingest_event(&usage(200, 75, None, 0, 0));
    acc.ingest_event(&usage(150, 60, None, 0, 0));
    assert_eq!(acc.tokens(), 275);
}

#[test]
fn monotonic_max_on_cache_tokens() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(0, 0, None, 1000, 500));
    acc.ingest_event(&usage(0, 0, None, 1500, 200));
    acc.ingest_event(&usage(0, 0, None, 1200, 800));
    assert_eq!(acc.cache_read_input_tokens(), 1500);
    assert_eq!(acc.cache_creation_input_tokens(), 800);
}

#[test]
fn out_of_order_usage_takes_max_not_last() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(500, 200, Some(0.05), 0, 0));
    acc.ingest_event(&usage(100, 50, Some(0.01), 0, 0));
    // tokens: max(700, 150) = 700 — NOT last-wins
    assert_eq!(acc.tokens(), 700);
    // cost: last seen wins (agent-provided cost is authoritative)
    assert_eq!(acc.cost_usd(), 0.01);
}

#[test]
fn terminal_usage_caps_running_total() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(100, 50, Some(0.01), 0, 0));
    acc.ingest_event(&usage(1000, 200, Some(0.05), 0, 0));
    // Terminal with smaller numbers should NOT go backwards
    acc.ingest_event(&terminal(
        StopReason::EndOfTurn,
        Some(Usage {
            input_tokens: 500,
            output_tokens: 100,
            cost_usd: Some(0.03),
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        }),
    ));
    assert_eq!(acc.tokens(), 1200);
    assert!((acc.cost_usd() - 0.03).abs() < 1e-9); // last seen cost
    assert!(acc.finished());
}

#[test]
fn terminal_usage_can_grow_running_total() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(100, 50, None, 0, 0));
    // Terminal with bigger numbers — claude-code sends one big final value
    acc.ingest_event(&terminal(
        StopReason::EndOfTurn,
        Some(Usage {
            input_tokens: 5000,
            output_tokens: 1000,
            cost_usd: Some(0.187),
            cache_read_input_tokens: 1000,
            cache_creation_input_tokens: 500,
        }),
    ));
    assert_eq!(acc.tokens(), 6000);
    assert_eq!(acc.cache_read_input_tokens(), 1000);
    assert_eq!(acc.cache_creation_input_tokens(), 500);
    assert!((acc.cost_usd() - 0.187).abs() < 1e-9);
}

#[test]
fn post_terminal_usage_events_are_ignored() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&terminal(StopReason::EndOfTurn, None));
    // Any further Usage is ignored
    acc.ingest_event(&usage(999_999, 999_999, Some(99.99), 0, 0));
    assert_eq!(acc.tokens(), 0);
    assert!(!acc.has_cost());
}

#[test]
fn pricing_fallback_when_cost_is_none() {
    let pricing = TestPricing::new();
    let mut acc = UsageAccumulator::new(Some("claude-sonnet-4".into()));
    acc.ingest_event(&usage(1_000_000, 500_000, None, 0, 0));
    assert!(!acc.has_cost());
    acc.finalize(&pricing);
    assert!(acc.has_cost());
    // 1M input @ $3/M + 500k output @ $15/M = 3.0 + 7.5 = 10.5
    assert!((acc.cost_usd() - 10.5).abs() < 1e-9);
}

#[test]
fn pricing_fallback_returns_zero_for_free_model() {
    let pricing = TestPricing::new();
    let mut acc = UsageAccumulator::new(Some("free-model".into()));
    acc.ingest_event(&usage(1_000_000, 1_000_000, None, 0, 0));
    acc.finalize(&pricing);
    assert_eq!(acc.cost_usd(), 0.0);
    assert!(acc.has_cost());
}

#[test]
fn pricing_fallback_skipped_when_model_unknown() {
    let pricing = TestPricing::new();
    let mut acc = UsageAccumulator::new(Some("unknown-model".into()));
    acc.ingest_event(&usage(1_000_000, 1_000_000, None, 0, 0));
    acc.finalize(&pricing);
    assert!(!acc.has_cost());
    assert_eq!(acc.cost_usd(), 0.0);
}

#[test]
fn pricing_fallback_skipped_when_model_is_none() {
    let pricing = TestPricing::new();
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(1_000_000, 1_000_000, None, 0, 0));
    acc.finalize(&pricing);
    assert!(!acc.has_cost());
}

#[test]
fn agent_supplied_cost_beats_pricing_fallback() {
    let pricing = TestPricing::new();
    let mut acc = UsageAccumulator::new(Some("claude-sonnet-4".into()));
    acc.ingest_event(&usage(100, 50, Some(0.42), 0, 0));
    acc.finalize(&pricing);
    // Pricing would compute ~$0.001; agent's 0.42 wins
    assert!((acc.cost_usd() - 0.42).abs() < 1e-9);
}

#[test]
fn finalize_arc_works() {
    let pricing: Arc<dyn PricingTable> = Arc::new(TestPricing::new());
    let mut acc = UsageAccumulator::new(Some("free-model".into()));
    acc.ingest_event(&usage(100, 50, None, 0, 0));
    acc.finalize_arc(&pricing);
    assert_eq!(acc.cost_usd(), 0.0);
    assert!(acc.has_cost());
}

#[test]
fn finalize_is_idempotent() {
    let pricing = TestPricing::new();
    let mut acc = UsageAccumulator::new(Some("claude-sonnet-4".into()));
    acc.ingest_event(&usage(1_000_000, 0, None, 0, 0));
    acc.finalize(&pricing);
    let first = acc.cost_usd();
    // ingest more tokens after finalize — cost should NOT grow because
    // finalize was the lock-in point. (Subsequent Usage still records
    // tokens but not cost since finalize is one-shot.)
    acc.finalize(&pricing);
    assert_eq!(acc.cost_usd(), first);
}

#[test]
fn terminal_with_none_usage_just_marks_finished() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&usage(100, 50, Some(0.01), 0, 0));
    acc.ingest_event(&terminal(StopReason::MaxTokens, None));
    assert!(acc.finished());
    // Existing values preserved
    assert_eq!(acc.tokens(), 150);
    assert_eq!(acc.cost_usd(), 0.01);
}

#[test]
fn non_usage_events_are_ignored() {
    let mut acc = UsageAccumulator::new(None);
    acc.ingest_event(&AgentEvent::Text {
        delta: "hello".into(),
    });
    acc.ingest_event(&AgentEvent::Error {
        code: "x".into(),
        message: "y".into(),
        recoverable: false,
    });
    assert_eq!(acc.tokens(), 0);
    assert!(!acc.finished());
}
