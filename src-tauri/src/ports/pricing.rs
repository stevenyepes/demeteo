//! Pricing table port.
//!
//! Maps an agent's model name to a per-million-token input/output USD cost.
//! The v1 implementation is a hard-coded table (R1 "Done means" — see
//! `docs/REDESIGN_EXECUTION_PLAN.md`). A later phase will let the user
//! override entries from Preferences; the trait is the only thing
//! downstream code touches, so that swap is local.

/// USD price per million tokens, for one named model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelPrice {
    pub input_per_million: f64,
    pub output_per_million: f64,
}

impl ModelPrice {
    /// Free model (local Ollama, etc.). $0 on both sides.
    pub const FREE: ModelPrice = ModelPrice { input_per_million: 0.0, output_per_million: 0.0 };

    /// Compute the USD cost for `input_tokens` + `output_tokens`.
    pub fn cost_usd(&self, input_tokens: u64, output_tokens: u64) -> f64 {
        let i = (input_tokens as f64 / 1_000_000.0) * self.input_per_million;
        let o = (output_tokens as f64 / 1_000_000.0) * self.output_per_million;
        i + o
    }
}

/// Source of model → price lookups.
///
/// Implementations: [`HardcodedPricingTable`](crate::adapters::pricing::HardcodedPricingTable).
/// Tests should use a tiny in-memory table; production reads the hard-coded
/// defaults.
pub trait PricingTable: Send + Sync {
    /// Look up a model's price. Returns `None` for unknown models — callers
    /// must fall back to "unknown cost" rather than guessing $0, otherwise
    /// the per-feature telemetry under-reports.
    fn price_for(&self, model: &str) -> Option<ModelPrice>;

    /// List all known model names. Useful for the Preferences UI.
    fn known_models(&self) -> Vec<String>;
}
