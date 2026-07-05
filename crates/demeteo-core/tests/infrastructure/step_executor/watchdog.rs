//! Tests for the context-window watchdog pure-function math.
//!
//! The driver method `watchdog_breached` delegates to
//! `ExecutionDriver::watchdog_breached_pure` which we exercise
//! here without constructing a full driver.

use crate::adapters::step_executor::driver::ExecutionDriver;

#[test]
fn no_budget_means_no_check() {
    // Legacy / unknown model path: watchdog treats None as
    // "no data, skip the check".
    assert!(!ExecutionDriver::watchdog_breached_pure(50_000, None));
    assert!(!ExecutionDriver::watchdog_breached_pure(0, None));
}

#[test]
fn zero_cumulative_means_no_breach() {
    // First-turn safety: even with a 200K budget, 0 cumulative
    // tokens hasn't breached anything yet.
    assert!(!ExecutionDriver::watchdog_breached_pure(0, Some(200_000)));
}

#[test]
fn below_threshold_does_not_breach() {
    // 159_999 / 200_000 = 79.99% — under the 80% cutoff.
    assert!(!ExecutionDriver::watchdog_breached_pure(159_999, Some(200_000)));
}

#[test]
fn at_threshold_breaches() {
    // 160_000 / 200_000 = exactly 80% — inclusive breach.
    assert!(ExecutionDriver::watchdog_breached_pure(160_000, Some(200_000)));
}

#[test]
fn above_threshold_breaches() {
    assert!(ExecutionDriver::watchdog_breached_pure(161_000, Some(200_000)));
    assert!(ExecutionDriver::watchdog_breached_pure(200_000, Some(200_000)));
}

#[test]
fn different_budgets_proportionally() {
    // 128K model (gpt-4o family) — threshold is 102_400.
    assert!(!ExecutionDriver::watchdog_breached_pure(102_399, Some(128_000)));
    assert!(ExecutionDriver::watchdog_breached_pure(102_400, Some(128_000)));
    // 100K model (gemini-pro) — threshold is 80_000.
    assert!(!ExecutionDriver::watchdog_breached_pure(79_999, Some(100_000)));
    assert!(ExecutionDriver::watchdog_breached_pure(80_000, Some(100_000)));
}

#[test]
fn threshold_constant_is_80_percent() {
    // Pin the threshold so accidental changes are caught.
    assert!((ExecutionDriver::WATCHDOG_THRESHOLD - 0.80).abs() < 1e-9);
}
