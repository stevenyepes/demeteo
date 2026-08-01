//! Shared token / cost accumulator.
//!
//! The single point of truth that turns a stream of `AgentEvent::Usage` and
//! `AgentEvent::TurnComplete { usage }` events into one reliable
//! `(tokens, cost_usd, cache_read, cache_creation)` tuple per turn.
//!
//! ## Why a shared accumulator
//!
//! There are **two** wire shapes, and they need opposite arithmetic:
//!
//! | Shape                            | Event                                     | Rule           | Agents                          |
//! |----------------------------------|-------------------------------------------|----------------|---------------------------------|
//! | Cumulative snapshot for the turn | [`AgentEvent::Usage`] / `TurnComplete`    | monotonic max  | Claude Code, opencode, hermes   |
//! | Increment for one model request  | [`AgentEvent::UsageDelta`]                | sum            | pi                              |
//!
//! Maxing a snapshot stream is right because the counters only grow
//! (verified against Anthropic SDK `cost-tracking.md` and opencode's
//! `step_finish.tokens.total` convention), so a value below the running
//! maximum is a duplicate or a reorder and is ignored. Maxing an *increment*
//! stream is a silent undercount that scales with turn count — a 30-turn run
//! reports roughly one turn's tokens — which is why the shape is carried by
//! the event variant rather than assumed here.
//!
//! ## Cost fallback
//!
//! When the agent doesn't send `cost_usd` (or sends `None`), `finalize`
//! looks the model up in the [`PricingTable`] and computes a USD figure
//! from `input_tokens + output_tokens`. Cache tokens are NOT included —
//! `cost_usd` already prices them (cache reads at ~10% of base).

use std::sync::Arc;

use crate::domain::agent_event::{AgentEvent, Usage};
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

    /// Apply one event, using the arithmetic the event's shape demands
    /// (module docs). Snapshots max every numeric field and take the
    /// last `cost_usd` — the agent's own figure is more authoritative than
    /// a derived estimate. Increments add.
    ///
    /// After a `TurnComplete { usage: Some(_), .. }` is ingested, the
    /// accumulator ignores further usage events for this turn — the
    /// terminal snapshot is authoritative and any post-terminus events
    /// would be a parser bug. Set `finished = true` defensively.
    pub fn ingest_event(&mut self, event: &AgentEvent) {
        match event {
            AgentEvent::Usage(u) => {
                if self.finished {
                    return;
                }
                self.apply_usage(u);
            }
            AgentEvent::UsageDelta(u) => {
                if self.finished {
                    return;
                }
                self.add_usage(u);
            }
            AgentEvent::TurnComplete { usage, .. } => {
                if let Some(u) = usage {
                    self.apply_usage(u);
                }
                self.finished = true;
            }
            AgentEvent::Error { usage: Some(u), .. } => {
                if self.finished {
                    return;
                }
                self.apply_usage(u);
            }
            _ => {}
        }
    }

    fn apply_usage(&mut self, u: &Usage) {
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

    fn add_usage(&mut self, u: &Usage) {
        self.running_input_tokens = self.running_input_tokens.saturating_add(u.input_tokens);
        self.running_output_tokens = self.running_output_tokens.saturating_add(u.output_tokens);
        self.running_cache_read = self
            .running_cache_read
            .saturating_add(u.cache_read_input_tokens);
        self.running_cache_creation = self
            .running_cache_creation
            .saturating_add(u.cache_creation_input_tokens);
        if let Some(c) = u.cost_usd {
            self.running_cost = Some(self.running_cost.unwrap_or(0.0) + c);
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

    /// Uncached input tokens only — on cache-aware agents (claude-code and
    /// friends) this is the prompt remainder *not* served from or written to
    /// the prompt cache. Denominator material for the cache-hit ratio.
    pub fn input_tokens(&self) -> u64 {
        self.running_input_tokens
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
