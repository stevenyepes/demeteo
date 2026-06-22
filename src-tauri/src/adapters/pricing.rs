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

/// Build a model name → price map keyed by lower-case name.
fn default_prices() -> HashMap<String, ModelPrice> {
    let mut m: HashMap<String, ModelPrice> = HashMap::new();

    // Anthropic Claude (USD per 1M tokens, as of 2026).
    m.insert(
        "claude-opus-4".to_string(),
        ModelPrice {
            input_per_million: 15.00,
            output_per_million: 75.00,
        },
    );
    m.insert(
        "claude-sonnet-4".to_string(),
        ModelPrice {
            input_per_million: 3.00,
            output_per_million: 15.00,
        },
    );
    m.insert(
        "claude-haiku-4".to_string(),
        ModelPrice {
            input_per_million: 0.80,
            output_per_million: 4.00,
        },
    );
    // Legacy family aliases — same prices, broader matching.
    m.insert(
        "claude-3-5-sonnet".to_string(),
        ModelPrice {
            input_per_million: 3.00,
            output_per_million: 15.00,
        },
    );
    m.insert(
        "claude-3-opus".to_string(),
        ModelPrice {
            input_per_million: 15.00,
            output_per_million: 75.00,
        },
    );
    m.insert(
        "claude-3-haiku".to_string(),
        ModelPrice {
            input_per_million: 0.25,
            output_per_million: 1.25,
        },
    );

    // OpenAI GPT family.
    m.insert(
        "gpt-4o".to_string(),
        ModelPrice {
            input_per_million: 2.50,
            output_per_million: 10.00,
        },
    );
    m.insert(
        "gpt-4o-mini".to_string(),
        ModelPrice {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
    );
    m.insert(
        "o1".to_string(),
        ModelPrice {
            input_per_million: 15.00,
            output_per_million: 60.00,
        },
    );
    m.insert(
        "o1-mini".to_string(),
        ModelPrice {
            input_per_million: 3.00,
            output_per_million: 12.00,
        },
    );
    m.insert(
        "o3-mini".to_string(),
        ModelPrice {
            input_per_million: 1.10,
            output_per_million: 4.40,
        },
    );

    // Google Gemini.
    m.insert(
        "gemini-2.5-pro".to_string(),
        ModelPrice {
            input_per_million: 1.25,
            output_per_million: 10.00,
        },
    );
    m.insert(
        "gemini-pro".to_string(),
        ModelPrice {
            input_per_million: 0.50,
            output_per_million: 1.50,
        },
    );

    // Local / free.
    m.insert("ollama".to_string(), ModelPrice::FREE);
    m.insert("llama".to_string(), ModelPrice::FREE);
    m.insert("local".to_string(), ModelPrice::FREE);

    m
}

/// Static, app-bundled pricing table.
pub struct HardcodedPricingTable {
    by_name: HashMap<String, ModelPrice>,
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

impl PricingTable for HardcodedPricingTable {
    fn price_for(&self, model: &str) -> Option<ModelPrice> {
        let key = model.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        if let Some(p) = self.by_name.get(&key) {
            return Some(*p);
        }
        // Prefix fallback so `claude-3-5-sonnet-20241022` still resolves.
        for (name, price) in &self.by_name {
            if key.starts_with(name) {
                return Some(*price);
            }
        }
        None
    }

    fn known_models(&self) -> Vec<String> {
        let mut v: Vec<String> = self.by_name.keys().cloned().collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_known_model() {
        let t = HardcodedPricingTable::new();
        let p = t.price_for("claude-sonnet-4").unwrap();
        assert_eq!(p.input_per_million, 3.00);
        assert_eq!(p.output_per_million, 15.00);
    }

    #[test]
    fn prefix_match_claude_aliased() {
        let t = HardcodedPricingTable::new();
        // Newer dated snapshots of 3.5 Sonnet should still price as Sonnet.
        let p = t.price_for("claude-3-5-sonnet-20241022").unwrap();
        assert_eq!(p.input_per_million, 3.00);
    }

    #[test]
    fn case_insensitive() {
        let t = HardcodedPricingTable::new();
        assert!(t.price_for("Claude-Sonnet-4").is_some());
    }

    #[test]
    fn unknown_model_returns_none() {
        let t = HardcodedPricingTable::new();
        assert!(t.price_for("not-a-real-model-9000").is_none());
    }

    #[test]
    fn empty_model_returns_none() {
        let t = HardcodedPricingTable::new();
        assert!(t.price_for("   ").is_none());
    }

    #[test]
    fn free_model_costs_zero() {
        let t = HardcodedPricingTable::new();
        let p = t.price_for("ollama").unwrap();
        assert_eq!(p.cost_usd(1_000_000, 1_000_000), 0.0);
    }

    #[test]
    fn cost_calculation_proportional() {
        let t = HardcodedPricingTable::new();
        let p = t.price_for("gpt-4o").unwrap();
        // 1M input + 1M output at gpt-4o = 2.50 + 10.00 = 12.50 USD.
        assert!((p.cost_usd(1_000_000, 1_000_000) - 12.50).abs() < 1e-9);
        // 1k input + 0 output = 2.50 / 1000 = 0.0025 USD.
        assert!((p.cost_usd(1_000, 0) - 0.0025).abs() < 1e-9);
    }

    #[test]
    fn known_models_is_nonempty_and_sorted() {
        let t = HardcodedPricingTable::new();
        let v = t.known_models();
        assert!(
            v.len() >= 5,
            "R1 requires 5-10 known models, got {}",
            v.len()
        );
        let mut sorted = v.clone();
        sorted.sort();
        assert_eq!(v, sorted, "known_models() should be sorted");
    }
}
