use crate::services::RunnerServices;
use demeteo_core::domain::ids::StepExecutionId;
use demeteo_core::ports::db::FeatureRepository;
use demeteo_core::ports::runner_run::{RunnerRun, RunnerRunPort};
use std::sync::Arc;

/// MC-D2/D3 ownership choke point: load `run_id` and confirm `client_id`
/// owns it. A run owned by a *different* client is reported as "no such
/// run" — byte-for-byte the same error a genuinely-absent run yields (see
/// [`no_such_run`]) — so ownership leaks no existence signal: a client
/// can't distinguish "that id isn't yours" from "that id doesn't exist".
///
/// Every run-scoped RPC funnels through this single helper so no method
/// can forget the check (P0.4's integration test iterates the whole
/// surface for a second client to prove it). `""`-vs-`""` (two legacy
/// clients, or a legacy run) matches — the documented single legacy
/// tenant (docs/MULTI_CLIENT_RUNNER.md Risk §7.1), not a boundary.
pub(super) fn require_owner(
    svc: &Arc<RunnerServices>,
    run_id: &str,
    client_id: &str,
) -> Result<RunnerRun, String> {
    check_owner(svc.ctx.runner_runs.get(run_id)?, run_id, client_id)
}

/// The pure ownership decision behind [`require_owner`], split out so the
/// leak-nothing property is unit-testable without a full `RunnerServices`:
/// a wrong-owner run and an absent run **must** yield the byte-identical
/// error (otherwise a client could probe another's run ids for existence).
fn check_owner(run: Option<RunnerRun>, run_id: &str, client_id: &str) -> Result<RunnerRun, String> {
    match run {
        Some(run) if run.owner_client_id == client_id => Ok(run),
        _ => Err(no_such_run(run_id)),
    }
}

/// The uniform "not here / not yours" error string. Kept as one function
/// so the absent-run and wrong-owner paths are guaranteed identical (a
/// drift between the two would re-open the existence-probe leak MC-D2
/// closes).
fn no_such_run(run_id: &str) -> String {
    format!("no such run: {}", run_id)
}

/// Resolve a bare step_execution_id — a `gate_id` (M5.3), a retry target
/// or an assignment target — to the run that owns its feature and confirm
/// `client_id` owns *that* run (MC-D2 / P0.4). Before multi-client, such an
/// id was a bearer capability: any tunnelled caller who learned one could
/// clear another client's parked gate. This closes it: resolve step →
/// feature → run, then owner-check the run. Every failure to resolve
/// (unknown step, orphan step, no run behind the feature, or a non-owner)
/// collapses to the *same* [`no_such_step`] error, so it leaks neither the
/// step's existence nor its ownership. Returns the owning run, which
/// `retry_step` needs to re-open.
///
/// Free over the two repositories it reads rather than taking
/// [`RunnerServices`], so every one of those refusals is reachable from a
/// test against a seeded database instead of an `AppContext`'s twenty-odd
/// ports (AGENTS.md §3).
pub(super) fn require_owner_of_step(
    features: &dyn FeatureRepository,
    runs: &dyn RunnerRunPort,
    step_execution_id: &str,
    client_id: &str,
) -> Result<RunnerRun, String> {
    let step = features
        .step_get(&StepExecutionId::from(step_execution_id.to_string()))?
        .ok_or_else(|| no_such_step(step_execution_id))?;
    let feature_id = step.feature_id.as_str();
    runs.list()?
        .into_iter()
        .find(|r| r.feature_id.as_deref() == Some(feature_id) && r.owner_client_id == client_id)
        .ok_or_else(|| no_such_step(step_execution_id))
}

/// The uniform "not here / not yours" error for a step id, kept as one
/// function for the reason [`no_such_run`] is: the absent-step, orphan-step
/// and wrong-owner paths — and a caller naming the wrong run for a step it
/// does own — must stay byte-identical, or the existence probe MC-D2 closes
/// re-opens.
pub(super) fn no_such_step(step_execution_id: &str) -> String {
    format!("no such step: {}", step_execution_id)
}

#[cfg(test)]
#[path = "ownership_tests.rs"]
mod tests;
