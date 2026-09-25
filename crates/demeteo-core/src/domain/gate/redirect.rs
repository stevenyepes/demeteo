//! Where a `redirect` gate decision lands.

use crate::domain::ids::StepId;
use crate::domain::models::StepConfig;

/// Free text longer than this reads as a report a reviewer attached, not an
/// address they typed. Long enough for a real instruction ("redo
/// s-tickets, the split is missing the delete case and the empty-state
/// copy"); short enough that a critic report — which routinely runs to
/// thousands of characters and, while explaining a finding, can quote a
/// step id belonging to a workflow it is merely *discussing* — never
/// qualifies the whole-word scan below. Below this bound a reviewer's
/// deliberate one-liner still wins; above it, a mention is coincidence,
/// not an address, and priority 1 falls through instead of matching.
const MAX_ADDRESSED_FEEDBACK_LEN: usize = 300;

/// Resolve the redirect target for a `redirect` gate decision.
///
/// Priority:
///   1. Step ID in `feedback` (if it matches one of `steps`) — either the
///      whole trimmed feedback, or a whole word within a longer free-text
///      note (e.g. "redo s-tickets, the split is too coarse"), **so long as
///      the feedback is no longer than [`MAX_ADDRESSED_FEEDBACK_LEN`]**. A
///      pipeline can have more than one artifact-only predecessor ahead of
///      a gate (e.g. ticket decomposition followed by a spec step); a
///      reviewer who names the one they mean should land there even
///      without typing nothing else, rather than falling through to a
///      fallback that may guess the other one. **Subject to the same
///      producer hop as priority 3**: naming a `task_list_from` step
///      directly is not an escape hatch from it — entering that step
///      without its producer having regenerated the list replays the
///      stale whole decomposition regardless of how the redirect was
///      addressed. The same holds for any step *between* the producer and
///      its consumer (a review gate): landing there runs the consumer on
///      the list the producer wrote last cycle, so it hops too.
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
                steps
                    .iter()
                    .position(|s| feedback_names_step(cleaned, &s.id.0))
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

    let gate_idx = gate_step_index as usize;
    let explicit = explicit.map(|idx| {
        rework_producer_for(steps, idx)
            .or_else(|| rework_producer_spanning(steps, idx, gate_idx))
            .unwrap_or(idx)
    });

    explicit
        .or_else(|| on_failure.and_then(|id| steps.iter().position(|s| s.id == *id)))
        .or_else(|| implement_fallback(gate_step_index as usize))
        .or_else(|| predecessor_fallback(gate_step_index))
}

/// Whether a reviewer's `feedback` addresses the step `id`: the whole trimmed
/// feedback, or a whole word of feedback no longer than
/// [`MAX_ADDRESSED_FEEDBACK_LEN`]. Shared with the synthetic park
/// ([`crate::domain::step_park::resolve_park`]) so a human names a target the
/// same way at either kind of stop.
pub(crate) fn feedback_names_step(feedback: &str, id: &str) -> bool {
    let cleaned = feedback.trim();
    if cleaned == id {
        return true;
    }
    if cleaned.len() > MAX_ADDRESSED_FEEDBACK_LEN {
        return false;
    }
    // Whole-word search: a bare substring match would also fire on
    // "s-tickets2" or a step id that is a prefix of another, so split on
    // anything that isn't part of a kebab-case id.
    cleaned
        .split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_')
        .any(|token| token == id)
}

/// The index of the step that produces `from_index`'s task list, when that
/// producer can turn free-text feedback into a delta.
///
/// `None` — meaning "keep targeting `from_index`" — for a step with no
/// `task_list_from` binding, a binding naming a step this workflow does not
/// contain, or a producer that declares no `rework_prompt_template`. That
/// last one is the opt-in: without a rework template the producer would
/// answer with a whole fresh decomposition, so redirecting through it would
/// re-run the entire feature *and* pay for a planning turn to decide to.
fn rework_producer_for(steps: &[StepConfig], from_index: usize) -> Option<usize> {
    let source = steps[from_index]
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

/// Whether `gate_idx` sits strictly between a rework-opted producer and the
/// step that consumes its list — the position where a redirect landing
/// mid-cycle would otherwise replace the verdict that opened the cycle.
pub(crate) fn gate_in_rework_span(steps: &[StepConfig], gate_idx: usize) -> bool {
    (gate_idx + 1..steps.len())
        .filter_map(|consumer| rework_producer_for(steps, consumer))
        .any(|producer| producer < gate_idx)
}

/// The producer whose `task_list_from` edge spans `target`: some consumer
/// `c` before the gate reads its list from producer `p`, with
/// `p < target <= c`. Everything in that span re-runs the consumer without
/// re-running the producer, which replays last cycle's list. Where spans
/// nest, the consumer closest to the gate wins — it is the one whose
/// output the gate was judging.
fn rework_producer_spanning(steps: &[StepConfig], target: usize, gate_idx: usize) -> Option<usize> {
    (0..gate_idx.min(steps.len()))
        .rev()
        .filter_map(|consumer| {
            let producer = rework_producer_for(steps, consumer)?;
            (producer < target && target <= consumer).then_some(producer)
        })
        .next()
}

#[cfg(test)]
#[path = "../../../tests/domain/gate/redirect.rs"]
mod tests;
