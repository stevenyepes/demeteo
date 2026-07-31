//! The per-turn dollar ceiling: what a run may spend on one agent turn.

/// Engine default per-turn dollar budget when neither the run
/// (`Feature::max_budget_usd`) nor the project
/// (`ProjectSettings::default_max_budget_usd`) sets one. This is the
/// *base* ceiling for the primary coding turn — generous enough that only
/// a true runaway trips it (the context watchdog resets long-running
/// sessions well before then), while still capping open-ended spend.
pub const DEFAULT_MAX_BUDGET_USD: f64 = 20.0;

/// Fractions of the resolved base budget granted to each bounded role
/// turn. These mirror the anti-runaway posture of the per-role
/// `max_turns` caps: a single-purpose turn that only interprets inlined
/// input into one answer should never approach the coding turn's spend.
/// At the $20 default these resolve to ~$0.50 / $2 / $5 / $8.
pub const BUDGET_FRACTION_TRIAGE: f64 = 0.025;
pub const BUDGET_FRACTION_FINALIZE: f64 = 0.10;
pub const BUDGET_FRACTION_VERIFIER: f64 = 0.25;
pub const BUDGET_FRACTION_PLANNER: f64 = 0.40;

/// The resolved *base* per-turn dollar budget for this run: the per-run
/// override, else the project default, else the engine default. Always
/// `Some` — every turn carries a ceiling (see
/// [`DEFAULT_MAX_BUDGET_USD`]).
pub fn base_max_budget_usd(run_override: Option<f64>, project_default: Option<f64>) -> f64 {
    run_override
        .or(project_default)
        .unwrap_or(DEFAULT_MAX_BUDGET_USD)
}

/// The per-turn dollar ceiling for a role turn, as `fraction` of the
/// resolved base budget. Pass `1.0` for the primary coding turn.
pub fn role_max_budget_usd(base: f64, fraction: f64) -> Option<f64> {
    Some(base * fraction)
}

#[cfg(test)]
#[path = "../../../tests/domain/agent_session/budget.rs"]
mod tests;
