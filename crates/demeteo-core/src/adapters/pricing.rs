//! Hard-coded pricing table.
//!
//! Per system pricing specifications:
//!
//! > The `PricingTable` is hard-coded with the 5–10 most common models
//! > (Claude Sonnet/Opus/Haiku, GPT-4o/o1/o3-mini, Gemini Pro, Llama via
//! > Ollama at $0). Editable from Preferences in a later phase.
//!
//! Prices are USD per million tokens. Match is case-insensitive on the
//! full model name, and by prefix for `claude-*` and `gpt-*` family
//! aliases (e.g. `claude-3-5-sonnet-20241022` resolves to Sonnet 4).

use std::collections::HashMap;

use crate::ports::pricing::{ModelPrice, PricingTable};

/// One row in the bundled pricing table: per-million USD for input +
/// output tokens, plus the model's known context-window size in
/// tokens (the budget the watchdog compares `cumulative_tokens`
/// against).
#[derive(Debug, Clone, Copy)]
struct PricingRow {
    price: ModelPrice,
    /// Input + output token budget the model exposes. `None` for
    /// free / local models where there's no enforced window (or we
    /// don't track one).
    context_window: Option<u64>,
}

const CONTEXT_1M: Option<u64> = Some(1_000_000);
const CONTEXT_400K: Option<u64> = Some(400_000);
const CONTEXT_272K: Option<u64> = Some(272_000);
const CONTEXT_200K: Option<u64> = Some(200_000);
const CONTEXT_128K: Option<u64> = Some(128_000);
const CONTEXT_100K: Option<u64> = Some(100_000);

/// Build a model name → price map keyed by lower-case name.
fn default_prices() -> HashMap<String, PricingRow> {
    let mut m: HashMap<String, PricingRow> = HashMap::new();

    // Anthropic Claude (USD per 1M tokens, as of 2026).
    // Claude Fable 5 — Anthropic's most capable model (GA); 1M context window,
    // $10/$50 per 1M. Keyed by the full id and the `fable` CLI alias so both
    // `claude-fable-5` (opencode/hermes routing) and a claude-code `--model
    // fable` selection resolve. The `claude-fable` prefix catches dated variants.
    m.insert(
        "claude-fable-5".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 10.00,
                output_per_million: 50.00,
            },
            context_window: CONTEXT_1M,
        },
    );
    m.insert(
        "fable".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 10.00,
                output_per_million: 50.00,
            },
            context_window: CONTEXT_1M,
        },
    );
    m.insert(
        "claude-opus-4".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 15.00,
                output_per_million: 75.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    m.insert(
        "claude-sonnet-4".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 3.00,
                output_per_million: 15.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    m.insert(
        "claude-haiku-4".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 0.80,
                output_per_million: 4.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    // Legacy family aliases — same prices, broader matching.
    m.insert(
        "claude-3-5-sonnet".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 3.00,
                output_per_million: 15.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    m.insert(
        "claude-3-opus".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 15.00,
                output_per_million: 75.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    m.insert(
        "claude-3-haiku".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 0.25,
                output_per_million: 1.25,
            },
            context_window: CONTEXT_200K,
        },
    );

    // OpenAI GPT family.
    m.insert(
        "gpt-4o".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 2.50,
                output_per_million: 10.00,
            },
            context_window: CONTEXT_128K,
        },
    );
    m.insert(
        "gpt-4o-mini".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 0.15,
                output_per_million: 0.60,
            },
            context_window: CONTEXT_128K,
        },
    );
    m.insert(
        "o1".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 15.00,
                output_per_million: 60.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    m.insert(
        "o1-mini".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 3.00,
                output_per_million: 12.00,
            },
            context_window: CONTEXT_128K,
        },
    );
    m.insert(
        "o3-mini".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 1.10,
                output_per_million: 4.40,
            },
            context_window: CONTEXT_200K,
        },
    );

    // OpenAI GPT-5 / Codex family (Codex CLI's default model line, Epic A1).
    // The `gpt-5` prefix row is the catch-all: `gpt-5.5`, `gpt-5.1-codex`,
    // `gpt-5-codex`, etc. all resolve to it via the prefix fallback in
    // `price_for`, so a Codex model bump costs correctly without a new row.
    m.insert(
        "gpt-5".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 1.25,
                output_per_million: 10.00,
            },
            context_window: CONTEXT_400K,
        },
    );
    m.insert(
        "gpt-5-mini".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 0.25,
                output_per_million: 2.00,
            },
            context_window: CONTEXT_400K,
        },
    );

    // pi routes through `provider/model` ids, so `openai-codex/gpt-5.6-luna`
    // never reaches the bare `gpt-5` prefix row and the context-window
    // watchdog would silently run with no budget. These two rows are keyed on
    // the provider-qualified form pi actually emits, and they differ from the
    // bare `gpt-5` row: `pi --list-models` reports 272K for this line, not the
    // 400K the direct-API models expose. `gpt-5.3-codex-spark` is the one
    // member at 128K, so it needs an exact row of its own — every other
    // `openai-codex/gpt-5*` resolves through the prefix row. Cost mirrors the
    // bare `gpt-5` row; pi reports `cost` on the wire, so this is a fallback
    // that is rarely consulted.
    m.insert(
        "openai-codex/gpt-5".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 1.25,
                output_per_million: 10.00,
            },
            context_window: CONTEXT_272K,
        },
    );
    m.insert(
        "openai-codex/gpt-5.3-codex-spark".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 1.25,
                output_per_million: 10.00,
            },
            context_window: CONTEXT_128K,
        },
    );

    // Google Gemini.
    m.insert(
        "gemini-2.5-pro".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 1.25,
                output_per_million: 10.00,
            },
            context_window: CONTEXT_200K,
        },
    );
    m.insert(
        "gemini-pro".to_string(),
        PricingRow {
            price: ModelPrice {
                input_per_million: 0.50,
                output_per_million: 1.50,
            },
            context_window: CONTEXT_100K,
        },
    );

    // Local / free — no enforced window; watchdog skips.
    m.insert(
        "ollama".to_string(),
        PricingRow {
            price: ModelPrice::FREE,
            context_window: None,
        },
    );
    m.insert(
        "llama".to_string(),
        PricingRow {
            price: ModelPrice::FREE,
            context_window: None,
        },
    );
    m.insert(
        "local".to_string(),
        PricingRow {
            price: ModelPrice::FREE,
            context_window: None,
        },
    );

    m
}

/// Static, app-bundled pricing table.
pub struct HardcodedPricingTable {
    by_name: HashMap<String, PricingRow>,
}

impl HardcodedPricingTable {
    pub fn new() -> Self {
        Self {
            by_name: default_prices(),
        }
    }
}

impl Default for HardcodedPricingTable {
    fn default() -> Self {
        Self::new()
    }
}

impl HardcodedPricingTable {
    /// Resolve the row backing `model`, accepting both a bare id
    /// (`gpt-5.6-luna`) and the `provider/model` form — which pi returns from
    /// `--list-models`, and which `agent_probe`'s opencode/hermes fallback
    /// list has always shipped.
    ///
    /// The qualified key is tried whole *before* it is stripped, so a row
    /// keyed on `openai-codex/gpt-5` keeps precedence over the bare `gpt-5`
    /// it would otherwise strip down to. Those two disagree — 272K against
    /// 400K — and the qualified one is what the agent actually reports.
    fn row_for(&self, model: &str) -> Option<&PricingRow> {
        let key = model.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        self.row_for_key(&key).or_else(|| {
            key.split_once('/')
                .and_then(|(_provider, bare)| self.row_for_key(bare))
        })
    }

    /// Exact hit, else the **longest** row name `key` starts with.
    ///
    /// Longest rather than first: `gpt-5-mini-2025-08-07` prefix-matches both
    /// `gpt-5` and `gpt-5-mini`, and `HashMap` iteration order is arbitrary,
    /// so returning the first match billed that model at whichever row the
    /// hasher happened to yield — five times over when `gpt-5` won.
    fn row_for_key(&self, key: &str) -> Option<&PricingRow> {
        if let Some(row) = self.by_name.get(key) {
            return Some(row);
        }
        self.by_name
            .iter()
            .filter(|(name, _)| key.starts_with(name.as_str()))
            .max_by_key(|(name, _)| name.len())
            .map(|(_name, row)| row)
    }
}

impl PricingTable for HardcodedPricingTable {
    fn price_for(&self, model: &str) -> Option<ModelPrice> {
        self.row_for(model).map(|row| row.price)
    }

    fn context_window(&self, model: &str) -> Option<u64> {
        self.row_for(model).and_then(|row| row.context_window)
    }

    fn known_models(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_name.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
#[path = "../../tests/infrastructure/pricing.rs"]
mod tests;
