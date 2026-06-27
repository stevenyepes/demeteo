//! Shared token / cost accumulator.
//!
//! The single point of truth that turns a stream of `AgentEvent::Usage` and
//! `AgentEvent::TurnComplete { usage }` events into one reliable
//! `(tokens, cost_usd, cache_read, cache_creation)` tuple per turn.
//!
//! ## Why a shared accumulator
//!
//! Each agent emits usage differently:
//!
//! | Agent        | Shape on the wire                                     |
//! |--------------|-------------------------------------------------------|
//! | Claude Code  | One terminal `result` event with `usage` + `cost_usd` |
//! | opencode     | Multiple `usage_update` + `step_finish` events         |
//! | hermes       | Multiple `usage_update` events                         |
//! | antigravity  | (no usage — out of scope)                              |
//!
//! Naively overwriting `latest_tokens` on each event loses or duplicates
//! data; naive summing overcounts. This module applies one rule:
//! **monotonic max**. The wire protocols are documented as cumulative per
//! turn (verified against Anthropic SDK `cost-tracking.md` and opencode's
//! `step_finish.tokens.total` convention). Any value strictly less than the
//! running maximum is ignored.
//!
//! ## Cost fallback
//!
//! When the agent doesn't send `cost_usd` (or sends `None`), `finalize`
//! looks the model up in the [`PricingTable`] and computes a USD figure
//! from `input_tokens + output_tokens`. Cache tokens are NOT included —
//! `cost_usd` already prices them (cache reads at ~10% of base).

use std::sync::Arc;

use crate::domain::agent_event::AgentEvent;
use crate::ports::pricing::PricingTable;

/// Accumulates token + cost telemetry from a single agent turn.
///
/// Pure data + pure logic — no I/O, no async, no locks. Construct one per
/// `stream_agent_turn` / verifier invocation; feed events via `ingest_event`;
/// finalize with the pricing table to get the resolved turn outcome.
#[derive(Debug, Clone, Default)]
pub struct UsageAccumulator {
    running_input_tokens: u64,
    running_output_tokens: u64,
    running_cost: Option<f64>,
    running_cache_read: u64,
    running_cache_creation: u64,
    finished: bool,
    model: Option<String>,
}

impl UsageAccumulator {
    pub fn new(model: Option<String>) -> Self {
        Self {
            model,
            ..Self::default()
        }
    }

    /// Apply one event. Monotonic max on every numeric field; cost is
    /// last-write-wins (the agent's `cost_usd` is the more authoritative
    /// figure than a derived estimate).
    ///
    /// After a `TurnComplete { usage: Some(_), .. }` is ingested, the
    /// accumulator ignores further `Usage` events for this turn — the
    /// terminal snapshot is authoritative and any post-terminus events
    /// would be a parser bug. Set `finished = true` defensively.
    pub fn ingest_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::Usage(u) => {
                if self.finished {
                    return;
                }
                self.running_input_tokens = self.running_input_tokens.max(u.input_tokens);
                self.running_output_tokens = self.running_output_tokens.max(u.output_tokens);
                self.running_cache_read = self.running_cache_read.max(u.cache_read_input_tokens);
                self.running_cache_creation = self
                    .running_cache_creation
                    .max(u.cache_creation_input_tokens);
                if let Some(c) = u.cost_usd {
                    self.running_cost = Some(c);
                }
            }
            AgentEvent::TurnComplete { usage, .. } => {
                if let Some(u) = usage {
                    self.running_input_tokens = self.running_input_tokens.max(u.input_tokens);
                    self.running_output_tokens = self.running_output_tokens.max(u.output_tokens);
                    self.running_cache_read =
                        self.running_cache_read.max(u.cache_read_input_tokens);
                    self.running_cache_creation = self
                        .running_cache_creation
                        .max(u.cache_creation_input_tokens);
                    if let Some(c) = u.cost_usd {
                        self.running_cost = Some(c);
                    }
                }
                self.finished = true;
            }
            _ => {}
        }
    }

    /// Resolve cost using the pricing table when the agent didn't supply it.
    ///
    /// Idempotent; safe to call multiple times. Does NOT mutate the input
    /// totals — those are already locked in by `ingest_event`.
    pub fn finalize(&mut self, pricing: &dyn PricingTable) {
        if self.running_cost.is_some() {
            return;
        }
        let Some(model) = self.model.as_deref() else {
            return;
        };
        let Some(price) = pricing.price_for(model) else {
            return;
        };
        self.running_cost =
            Some(price.cost_usd(self.running_input_tokens, self.running_output_tokens));
    }

    /// Resolve cost using a shared pricing handle (Arc). Convenience for
    /// call sites that already hold an `Arc<dyn PricingTable>`.
    pub fn finalize_arc(&mut self, pricing: &Arc<dyn PricingTable>) {
        self.finalize(pricing.as_ref());
    }

    pub fn tokens(&self) -> i64 {
        (self.running_input_tokens + self.running_output_tokens) as i64
    }

    pub fn cost_usd(&self) -> f64 {
        self.running_cost.unwrap_or(0.0)
    }

    pub fn cache_read_input_tokens(&self) -> u64 {
        self.running_cache_read
    }

    pub fn cache_creation_input_tokens(&self) -> u64 {
        self.running_cache_creation
    }

    pub fn has_cost(&self) -> bool {
        self.running_cost.is_some()
    }

    pub fn finished(&self) -> bool {
        self.finished
    }

    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }
}

#[cfg(test)]
#[path = "../../tests/unit/usage_accumulator.rs"]
mod tests;
