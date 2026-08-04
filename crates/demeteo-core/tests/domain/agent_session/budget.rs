// Tests extracted from `crates/demeteo-core/src/domain/agent_session/budget.rs`
// (mirrored-tests convention). `super` = that module.

use super::*;

/// Run override, else project default, else the engine default — the whole
/// precedence chain, which used to be unreachable because it read two fields
/// of a struct carrying twenty ports it never touched.
#[test]
fn base_max_budget_usd_precedence() {
    assert_eq!(base_max_budget_usd(Some(7.5), Some(3.0)), 7.5);
    assert_eq!(base_max_budget_usd(None, Some(3.0)), 3.0);
    assert_eq!(base_max_budget_usd(None, None), 20.0);
    assert_eq!(base_max_budget_usd(Some(7.5), None), 7.5);
}

/// A per-run override of zero is a real ceiling, not "unset": `Option::or`
/// keeps it, and a run that asked to spend nothing must not silently inherit
/// the $20 default.
#[test]
fn a_zero_override_is_a_ceiling_not_an_absence() {
    assert_eq!(base_max_budget_usd(Some(0.0), Some(3.0)), 0.0);
    assert_eq!(base_max_budget_usd(None, Some(0.0)), 0.0);
}

#[test]
fn role_max_budget_usd_scales_base() {
    assert_eq!(role_max_budget_usd(20.0, 1.0), Some(20.0));
    assert_eq!(role_max_budget_usd(8.0, 0.25), Some(2.0));
    assert_eq!(role_max_budget_usd(0.0, 0.4), Some(0.0));
}

/// The claim the fraction constants' doc makes: at the $20 default the four
/// bounded role turns resolve to ~$0.50 / $2 / $5 / $8.
#[test]
fn the_documented_role_ceilings_hold_at_the_engine_default() {
    let base = base_max_budget_usd(None, None);
    let at = |f: f64| role_max_budget_usd(base, f).unwrap_or(f64::NAN);
    assert!((at(BUDGET_FRACTION_TRIAGE) - 0.50).abs() < 1e-9);
    assert!((at(BUDGET_FRACTION_FINALIZE) - 2.0).abs() < 1e-9);
    assert!((at(BUDGET_FRACTION_VERIFIER) - 5.0).abs() < 1e-9);
    assert!((at(BUDGET_FRACTION_PLANNER) - 8.0).abs() < 1e-9);
}

/// Ordering the roles rely on: triage is the cheapest turn and the coding turn
/// is the only one at full base.
#[test]
fn the_bounded_roles_stay_below_the_coding_turn() {
    const {
        assert!(BUDGET_FRACTION_TRIAGE < BUDGET_FRACTION_FINALIZE);
        assert!(BUDGET_FRACTION_FINALIZE < BUDGET_FRACTION_VERIFIER);
        assert!(BUDGET_FRACTION_VERIFIER < BUDGET_FRACTION_PLANNER);
        assert!(BUDGET_FRACTION_PLANNER < 1.0);
    }
}
