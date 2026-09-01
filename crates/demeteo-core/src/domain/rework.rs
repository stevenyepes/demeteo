//! Why a step is being entered *again*, and what that means for its output.
//!
//! A step that produces a task list is re-entered for two very different
//! reasons, and until now nothing told it which:
//!
//! * A reviewer at a gate, or the step's own retry, said **the decomposition
//!   is wrong** — too coarse, mis-ordered, a ticket that needs splitting. The
//!   right response is a revised *whole* list. Nothing has been implemented.
//! * A validator or critic **downstream of the step that executed the list**
//!   rejected the result. The right response is a small *delta* list naming
//!   only what the verdict rejected — because every ticket's code is already
//!   committed on the feature branch, and re-emitting the original
//!   decomposition means paying for all of it again.
//!
//! Both arrive as the same `RetryContext`, so the two used to be
//! indistinguishable and the second was answered as if it were the first.
//! That is the whole cost this module exists to remove: a 25-ticket feature
//! whose validator flagged four defects re-ran all 25 tickets, twice.
//!
//! # The question is about the *consumer*, not the producer
//!
//! The tempting rule — "the failing step is downstream of me, so my output
//! has been implemented" — is wrong, and the shipped pipeline is the
//! counterexample:
//!
//! ```text
//! research → spec → tickets → gate-review → implement → validate → critic
//! ```
//!
//! `gate-review` is downstream of `tickets`, but it sits *in front of* the
//! step that executes the list. A reviewer rejecting the decomposition there
//! has rejected a plan, not an implementation: the branch carries nothing,
//! and answering with a delta list would emit tickets that "fix" code which
//! was never written.
//!
//! What actually distinguishes the two is whether the failure came from
//! behind the **consumer** — the `sequence` step whose `task_list_from`
//! names this producer. Only a step downstream of *that* can have observed
//! an implementation, because the consumer must have completed for control
//! to have reached it. So the classification is one reachability query,
//! rooted at the consumer. No step kind, no capability, no step-id
//! convention: any workflow that puts a verdict step behind a sequence step
//! gets rework semantics for free.

use crate::domain::ids::StepId;
use crate::domain::models::StepConfig;
use crate::domain::workflow_graph::WorkflowGraph;

/// Why this step is running, from the point of view of what it should emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReworkMode {
    /// First pass. No prior failure — produce the full decomposition.
    Greenfield,
    /// Re-entered because this step's output was rejected before anything
    /// was built from it: its own retry, or a reviewer at a gate between it
    /// and the step that executes the list. The branch carries no
    /// implementation of it. Revise the whole list.
    Revision,
    /// Re-entered because a step downstream of the one that *consumed* this
    /// step's output rejected the work built from it. That work is on the
    /// feature branch. Emit only what closes the verdict.
    Rework,
}

impl ReworkMode {
    /// Wire form, for prompts, logs, and the `{{rework_mode}}` placeholder.
    pub fn as_str(self) -> &'static str {
        match self {
            ReworkMode::Greenfield => "greenfield",
            ReworkMode::Revision => "revision",
            ReworkMode::Rework => "rework",
        }
    }

    /// True for [`Self::Rework`] — the one mode that changes what a step
    /// emits rather than merely how it revises it.
    pub fn is_rework(self) -> bool {
        matches!(self, ReworkMode::Rework)
    }
}

/// The part of a retry context this decision reads.
///
/// Deliberately not the adapter's `RetryContext`: that type is `pub(crate)`
/// to the executor and carries prompt-rendering fields (feedback prose,
/// attempt counters) this decision must not consult. One field decides it,
/// and naming exactly that keeps the rule honest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryOrigin<'a> {
    /// Step whose failure opened this loop iteration.
    pub failing_step_id: &'a str,
    /// 1-based attempt now starting. Not part of the classification — a
    /// rework cycle is a rework cycle whether it is the first or the third
    /// — but carried so callers can render `{{rework_cycle}}` from the one
    /// value that is already authoritative.
    pub iteration: u32,
}

/// The `sequence` step that executes `producer`'s task list, if any.
///
/// A producer with no consumer cannot be in rework mode: nothing in the
/// workflow turns its output into commits, so there is never an
/// implementation to emit a delta against. First match wins — a second
/// consumer of the same list is a shape no starter has and no builder can
/// draw, and picking the earlier one keeps the answer deterministic.
pub fn task_list_consumer<'a>(steps: &'a [StepConfig], producer: &StepId) -> Option<&'a StepId> {
    steps
        .iter()
        .find(|s| {
            s.task_list_from
                .as_ref()
                .is_some_and(|src| !src.0.is_empty() && src == producer)
        })
        .map(|s| &s.id)
}

/// Classify why `this_node` is running.
///
/// `origin` is `None` on a fresh run, which is [`ReworkMode::Greenfield`]
/// and the only way to reach it. `consumer` is the step that executes this
/// node's task list ([`task_list_consumer`]); `None` for a node whose
/// output nothing implements.
///
/// [`ReworkMode::Rework`] requires the failing step to be a **strict
/// descendant of the consumer**. Everything else is
/// [`ReworkMode::Revision`]: this node failing on its own, a gate between
/// this node and the consumer redirecting into it, the consumer itself
/// failing (which rolls its own commits back on the way out), a node the
/// graph does not contain, and a producer with no consumer at all.
///
/// The asymmetry is deliberate. Revision re-emits the whole list, which is
/// always *correct* and only ever wasteful. Rework skips work on the claim
/// that it is already committed, so every uncertain input resolves away
/// from it.
pub fn classify(
    graph: &WorkflowGraph,
    this_node: &StepId,
    consumer: Option<&StepId>,
    origin: Option<RetryOrigin<'_>>,
) -> ReworkMode {
    let Some(origin) = origin else {
        return ReworkMode::Greenfield;
    };
    let Some(consumer) = consumer else {
        return ReworkMode::Revision;
    };
    // The empty string is the synthesized "clear after the next completed
    // step" context (per-task retry notes), which names no failing step and
    // therefore cannot be shown to be downstream of anything.
    let failing = origin.failing_step_id.trim();
    if failing.is_empty() || failing == this_node.0 {
        return ReworkMode::Revision;
    }
    let failing = StepId::from(failing.to_string());
    if graph.is_ancestor(consumer, &failing) {
        ReworkMode::Rework
    } else {
        ReworkMode::Revision
    }
}

/// Whether completing `completed_step_id` closes the retry loop that
/// `failing_step_id` opened, making the carried feedback stale.
///
/// Retry feedback lives until the step that originally failed succeeds.
/// Intermediate steps — the redirect target and everything between it and
/// the failing step — all see it; once the failing step passes, the loop is
/// closed.
///
/// `None` is no retry in flight (nothing to close). An empty
/// `failing_step_id` is the legacy "clear after the next completed step"
/// shape and still means closed, so a pre-P1.10 row does not pin feedback
/// to a step id that was never recorded.
///
/// Pure because more than one path completes a step — the ordinary one and
/// a human approving a park — and a rule spelled once per caller is a rule
/// that drifts. A park raised by the failing step closes its own loop, and
/// getting that wrong leaks a previous cycle's feedback into every prompt
/// after it.
pub fn retry_loop_closed(failing_step_id: Option<&str>, completed_step_id: &str) -> bool {
    match failing_step_id {
        None => true,
        Some(failing) => failing.is_empty() || failing == completed_step_id,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/rework.rs"]
mod tests;
