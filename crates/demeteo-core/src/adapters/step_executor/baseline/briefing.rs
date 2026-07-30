//! Assembling the `{{harness_baseline}}` prompt block — the two lookups the
//! pure wording cannot do for itself.

use crate::domain::models::{Feature, StepConfig};
use crate::ports::db::ProjectRepository;

/// Render the `{{harness_baseline}}` prompt block for this run: which gates
/// will judge the finished work, and what each already said about this
/// repository.
///
/// The gate list is resolved through
/// [`resolve_harnesses`](crate::domain::verifier::resolve_harnesses) — the
/// same chain validate itself resolves through — over the declarations of
/// **every step in this workflow that carries a verifier**, deduplicated by
/// name. Asking the project alone would be wrong for a workflow whose
/// validate step pins its own gates, and telling `s-spec` about gates that
/// will not run is the same class of lie as telling it about none of them.
///
/// The wording is
/// [`render_harness_briefing`](crate::domain::harness_baseline::render_harness_briefing),
/// which is pure and lives in `domain/`; what happens here is only the two
/// lookups it cannot do. Anything unreadable yields an empty block rather than
/// a guess: a prompt section that describes a harness this project does not
/// have is worse than no section.
///
/// A free function over the one port it needs rather than a method on
/// `ExecutionDriver`, which carries twenty-odd ports this never reads
/// (AGENTS.md §3). The two plain values it also needs — the workflow's steps
/// and the harness ceiling — are already resolved by the time anyone asks.
pub(crate) fn harness_briefing(
    projects: &dyn ProjectRepository,
    steps: &[StepConfig],
    ceiling_s: u64,
    feature: Option<&Feature>,
) -> String {
    let Some(feature) = feature else {
        return String::new();
    };
    let Some(settings) = projects.get_settings(&feature.project_id).ok().flatten() else {
        return String::new();
    };

    let gates = crate::domain::verifier::resolve_gating_harnesses(
        steps,
        &settings.worktree_strategy,
        ceiling_s,
    );

    crate::domain::harness_baseline::render_harness_briefing(
        &gates,
        feature.harness_baseline.as_ref(),
    )
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/baseline/briefing.rs"]
mod tests;
