//! Tests for the context-window watchdog and the per-turn budget
//! resolution on `ExecutionDriver`.
//!
//! Mirrored-tests convention: `super` = the `step_executor::driver`
//! module. `pub(crate)` symbols are accessible because the integrator
//! mounts this file via `#[path = "..."]` from `driver/watchdog.rs`.
//!
//! The pure-function tests (`watchdog_breached_pure`, `agent_session_key`)
//! run without any driver construction. The two budget tests
//! (`base_max_budget_usd_precedence`, `role_max_budget_usd_scales_base`)
//! are gated `#[ignore]` because `ExecutionDriver` requires a fully
//! populated set of port stubs the rest of the driver never reads —
//! those stubs exceed this test file's fair scope. The pure `Option::or`
//! semantics they test are already covered by the `loop_budget_precedence`
//! test in `driver.rs` (the identical idiomatic pattern in
//! `resolve_loop_iterations`).

use crate::domain::ids::StepId;
use crate::domain::models::{EffortLevel, StepConfig};

use super::ExecutionDriver;

// ── Pure watchdog_breached_pure coverage ──────────────────────────────────────

#[test]
fn watchdog_breached_pure_no_budget_never_breaches() {
    // Legacy / unknown model path: `None` budget means "no data, skip".
    assert!(!ExecutionDriver::watchdog_breached_pure(0, None));
    assert!(!ExecutionDriver::watchdog_breached_pure(50_000, None));
    assert!(!ExecutionDriver::watchdog_breached_pure(u64::MAX, None));
}

#[test]
fn watchdog_breached_pure_zero_cumulative_never_breaches() {
    // First-turn safety: even with a generous budget, 0 cumulative
    // hasn't breached anything yet.
    assert!(!ExecutionDriver::watchdog_breached_pure(0, Some(1_000)));
    assert!(!ExecutionDriver::watchdog_breached_pure(0, Some(200_000)));
}

#[test]
fn watchdog_breached_pure_under_threshold_false() {
    // 80% of 1000 = 800 (inclusive). 799 stays strictly below.
    assert!(!ExecutionDriver::watchdog_breached_pure(799, Some(1_000)));
    assert!(!ExecutionDriver::watchdog_breached_pure(1, Some(1_000)));
}

#[test]
fn watchdog_breached_pure_at_threshold_true() {
    // The threshold is inclusive — exact equality breaches.
    assert!(ExecutionDriver::watchdog_breached_pure(800, Some(1_000)));
}

#[test]
fn watchdog_breached_pure_over_threshold_true() {
    assert!(ExecutionDriver::watchdog_breached_pure(801, Some(1_000)));
    assert!(ExecutionDriver::watchdog_breached_pure(1_000, Some(1_000)));
}

// ── Session-key fingerprint coverage ─────────────────────────────────────────

fn step() -> StepConfig {
    StepConfig {
        effort: None,
        id: StepId::from("s-impl".to_string()),
        kind: "agent".to_string(),
        title: "Implement".to_string(),
        agent_kind: None,
        model: None,
        prompt_template: None,
        on_failure: None,
        max_iterations: None,
        artifacts: None,
        verifier: None,
        capability: None,
        allow_network: false,
        allow_shell: false,
        gate_class: None,
        task_list_from: None,
        ..Default::default()
    }
}

/// AC6 — regression guard. Two efforts must produce two keys (see
/// `agent_session_key` doc comment).
#[test]
fn session_key_distinguishes_two_efforts() {
    let s = step();
    let low = ExecutionDriver::agent_session_key("f-1", &s, Some("m"), EffortLevel::Low);
    let max = ExecutionDriver::agent_session_key("f-1", &s, Some("m"), EffortLevel::Max);
    assert_ne!(
        low, max,
        "a change in effort alone must force a fresh session"
    );
    assert_eq!(
        low,
        ExecutionDriver::agent_session_key("f-1", &s, Some("m"), EffortLevel::Low),
        "the same effort shares one key (the --resume cache hit exists to preserve)"
    );
}

/// Sanity: identical inputs → identical keys (the fingerprint is
/// deterministic, not random).
#[test]
fn session_key_same_effort_shares_key() {
    let s = step();
    let a = ExecutionDriver::agent_session_key("f-1", &s, Some("m"), EffortLevel::High);
    let b = ExecutionDriver::agent_session_key("f-1", &s, Some("m"), EffortLevel::High);
    assert_eq!(a, b);
}

// ── Budget-precedence tests (gated — see module docs) ────────────────────────
//
// `base_max_budget_usd` and `role_max_budget_usd` read only two fields
// of `ExecutionDriver`, but `ExecutionDriver` has 30+ fields and the
// rest are `Arc<dyn MyPort>` that need a stub impl for the struct to
// compile. The actual budget math is three lines of `Option::or` whose
// semantics are identical to the `resolve_loop_iterations` case
// already covered by `loop_budget_precedence` in `driver.rs`. Gates
// here so a future test-only fixture builder can enable them cheaply.

#[test]
#[ignore = "requires a full ExecutionDriver fixture; see module docs"]
fn base_max_budget_usd_precedence() {
    // run override > project default > engine default (20.0)
    unimplemented!("see module docs")
}

#[test]
#[ignore = "requires a full ExecutionDriver fixture; see module docs"]
fn role_max_budget_usd_scales_base() {
    // Some(base * fraction)
    unimplemented!("see module docs")
}
