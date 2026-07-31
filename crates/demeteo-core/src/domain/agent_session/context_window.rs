//! Has the session outgrown the model's context window?
//!
//! The Tier-1 "compact or reset" decision from the reliability plan, separated
//! from the mechanism that acts on it. The driver reads the live session's
//! cumulative token count, asks here, and on a `true` kills the session and
//! marks it dirty so the next step's `spawn_agent_session` re-spawns fresh
//! with a one-shot recap. Only the question is here; all of that acting stays
//! in `driver/watchdog.rs`.

/// The fraction of the model's context window at which the
/// watchdog resets the feature-wide agent session. Per the
/// Tier-1 plan: 80% leaves 20% headroom for the new turn's
/// growth and the in-flight prompt + tools.
pub const THRESHOLD: f64 = 0.80;

/// Returns `true` when `cumulative >= THRESHOLD × budget`. Returns `false`
/// when the budget is unknown (`None` — legacy behavior) or cumulative is
/// zero (first turn).
pub fn breached(cumulative: u64, budget: Option<u64>) -> bool {
    let Some(budget) = budget else {
        return false;
    };
    if cumulative == 0 {
        return false;
    }
    let threshold = ((budget as f64) * THRESHOLD) as u64;
    cumulative >= threshold
}

#[cfg(test)]
#[path = "../../../tests/domain/agent_session/context_window.rs"]
mod tests;
