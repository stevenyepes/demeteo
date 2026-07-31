// Tests extracted from `crates/demeteo-core/src/domain/agent_session/context_window.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn no_budget_never_breaches() {
    // Legacy / unknown model path: `None` budget means "no data, skip".
    assert!(!breached(0, None));
    assert!(!breached(50_000, None));
    assert!(!breached(u64::MAX, None));
}

#[test]
fn zero_cumulative_never_breaches() {
    // First-turn safety: even with a generous budget, 0 cumulative
    // hasn't breached anything yet.
    assert!(!breached(0, Some(1_000)));
    assert!(!breached(0, Some(200_000)));
}

#[test]
fn under_threshold_is_false() {
    // 159_999 / 200_000 = 79.99% — under the 80% cutoff.
    assert!(!breached(159_999, Some(200_000)));
    assert!(!breached(1, Some(200_000)));
}

#[test]
fn at_threshold_is_true() {
    // The threshold is inclusive — exact equality breaches.
    assert!(breached(160_000, Some(200_000)));
}

#[test]
fn over_threshold_is_true() {
    assert!(breached(161_000, Some(200_000)));
    assert!(breached(200_000, Some(200_000)));
}

/// The cutoff is a fraction of the model's own window, not a fixed token
/// count, so a smaller model resets proportionally earlier.
#[test]
fn different_budgets_scale_proportionally() {
    // 128K model (gpt-4o family) — threshold is 102_400.
    assert!(!breached(102_399, Some(128_000)));
    assert!(breached(102_400, Some(128_000)));
    // 100K model (gemini-pro) — threshold is 80_000.
    assert!(!breached(79_999, Some(100_000)));
    assert!(breached(80_000, Some(100_000)));
}

#[test]
fn threshold_constant_is_80_percent() {
    // Pin the threshold so accidental changes are caught.
    assert!((THRESHOLD - 0.80).abs() < 1e-9);
}
