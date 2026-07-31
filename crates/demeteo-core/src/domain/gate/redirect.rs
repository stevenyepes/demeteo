//! Where a `redirect` gate decision lands.

use crate::domain::ids::StepId;
use crate::domain::models::StepConfig;

/// Resolve the redirect target for a `redirect` gate decision.
///
/// Priority:
///   1. Step ID in `feedback` (if it matches one of `steps`) — either the
///      whole trimmed feedback, or a whole word within a longer free-text
///      note (e.g. "redo s-tickets, the split is too coarse"). A pipeline
///      can have more than one artifact-only predecessor ahead of a gate
///      (e.g. ticket decomposition followed by a spec step); a reviewer who
///      names the one they mean should land there even without typing
///      nothing else, rather than falling through to a fallback that may
///      guess the other one.
///   2. `on_failure` on the gate's step config.
///   3. The nearest preceding step whose effective capability is
///      `Implement` — **or, when that step reads its task list from a
///      producer that declares a `rework_prompt_template`, the producer
///      itself.** This is the natural intent of "give the agent my
///      feedback and redo it" — implementation feedback should land on a
///      step that can actually modify code. Without this rule, feedback at
///      `s-gate-ship` (index 6 in the standard pipeline) routes to
///      `s-validate` (index 5), which is a verify-only step that documents
///      findings but cannot write code, so the user's feedback just gets
///      logged into `validation-report.md` and bounced back to
///      `s-implement` via the verifier two iterations later.
///
///      The producer hop exists because a `sequence` step cannot *act* on
///      free-text feedback: it runs whatever list it is handed. Landing on
///      it means re-running the list it already ran — the whole feature,
///      re-implemented over itself, which is the cost the rework design
///      removes. Landing on the producer turns "the empty state looks
///      wrong" into two tickets. Gated on the producer declaring a rework
///      template, so a workflow that never opted in keeps the old target
///      and the old behaviour.
///   4. The step immediately before the gate — a safety net for
///      workflows that have no implement-capable step preceding
///      the gate (e.g. a pre-implementation review gate). Keeps
///      the pipeline from silently cancelling on free-text feedback.
///   5. `None` only when the gate is the very first step.
pub(crate) fn resolve_redirect_target(
    steps: &[StepConfig],
    on_failure: Option<&StepId>,
    gate_step_index: u32,
    feedback: Option<&str>,
) -> Option<usize> {
    let explicit = feedback
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .and_then(|cleaned| {
            steps.iter().position(|s| s.id.0 == cleaned).or_else(|| {
                // Whole-word search: a bare substring match would also fire
                // on "s-tickets2" or a step id that is a prefix of another,
                // so split on anything that isn't part of a kebab-case id.
                steps.iter().position(|s| {
                    cleaned
                        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
                        .any(|token| token == s.id.0)
                })
            })
        });

    let implement_fallback = |gate_idx: usize| -> Option<usize> {
        if gate_idx == 0 {
            return None;
        }
        let implementer = steps[..gate_idx].iter().rposition(|s| {
            s.effective_capability() == crate::domain::permission::StepCapability::Implement
        })?;
        Some(rework_producer_for(steps, implementer).unwrap_or(implementer))
    };

    let predecessor_fallback = |gate_idx: u32| -> Option<usize> {
        if gate_idx > 0 {
            Some(gate_idx as usize - 1)
        } else {
            None
        }
    };

    explicit
        .or_else(|| on_failure.and_then(|id| steps.iter().position(|s| s.id == *id)))
        .or_else(|| implement_fallback(gate_step_index as usize))
        .or_else(|| predecessor_fallback(gate_step_index))
}

/// The index of the step that produces `implementer`'s task list, when that
/// producer can turn free-text feedback into a delta.
///
/// `None` — meaning "keep targeting the implementer" — for a step with no
/// `task_list_from` binding, a binding naming a step this workflow does not
/// contain, or a producer that declares no `rework_prompt_template`. That
/// last one is the opt-in: without a rework template the producer would
/// answer with a whole fresh decomposition, so redirecting through it would
/// re-run the entire feature *and* pay for a planning turn to decide to.
fn rework_producer_for(steps: &[StepConfig], implementer: usize) -> Option<usize> {
    let source = steps[implementer]
        .task_list_from
        .as_ref()
        .filter(|s| !s.0.is_empty())?;
    let producer = steps.iter().position(|s| s.id == *source)?;
    steps[producer]
        .rework_prompt_template
        .as_deref()
        .filter(|t| !t.trim().is_empty())?;
    Some(producer)
}

#[cfg(test)]
#[path = "../../../tests/domain/gate/redirect.rs"]
mod tests;
