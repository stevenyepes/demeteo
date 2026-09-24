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
use crate::domain::models::{StepConfig, StepExecution};
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

/// Feedback captured when a step fails and the loop redirects back to an
/// earlier step. Injected into the retried step's prompt as
/// `{{retry_feedback}}` / `{{iteration}}` / `{{max_iterations}}` so the
/// retry isn't blind.
///
/// Mirrored to the `retry_contexts` table (V57) so a driver started after
/// a restart resumes inside the loop rather than outside it; see
/// [`restore_retry_context`] for what survives that trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryContext {
    /// Raw failure / verifier reason from the step that triggered the loop.
    pub feedback: String,
    /// 1-based attempt number we're now starting. Drives prompts only: the
    /// redirect budget is `step_executions.iteration_count`.
    pub iteration: u32,
    /// Effective max iterations for this loop.
    pub max: u32,
    /// Failing test identifiers from a structured verdict (empty for
    /// plain failures).
    ///
    /// Reaches a prompt twice, and the two are not redundant: as *prose*
    /// inside `feedback` (rendered by
    /// [`VerdictFailure`](crate::domain::verifier::VerdictFailure), which
    /// a template can only quote) and as the structured
    /// `{{failing_tests}}` bullets a rework template acts on — see the
    /// executor's `bind_rework_context`.
    pub failing_tests: Vec<String>,
    /// Repo-relative files a structured verdict implicated (empty for
    /// plain failures). Bound as `{{implicated_files}}` for a rework
    /// template, and — on the legacy planner-sourced path only — used to
    /// select which of a cached plan's tasks re-run.
    pub implicated_files: Vec<String>,
    /// Step id of the step whose failure opened this loop iteration.
    /// The feedback stays alive for *every* step between the redirect
    /// target and this step, and is cleared only when this step finally
    /// completes — so e.g. a re-run of `s-validate` still knows what it
    /// failed on last time instead of re-checking blind. Empty string
    /// means "clear after the next completed step" (legacy behavior,
    /// used by synthesized per-subtask contexts).
    pub failing_step_id: String,
}

impl Default for RetryContext {
    /// A context that is *not* a retry: no prior failure, one attempt.
    ///
    /// `iteration`/`max` are 1 rather than 0 because they are rendered
    /// straight into a prompt as "attempt {iteration} of {max}", and
    /// "attempt 0 of 0" describes a run that never happened.
    fn default() -> Self {
        Self {
            feedback: String::new(),
            iteration: 1,
            max: 1,
            failing_tests: Vec::new(),
            implicated_files: Vec::new(),
            failing_step_id: String::new(),
        }
    }
}

impl RetryContext {
    /// This context with its `feedback` replaced.
    ///
    /// For a task-scoped `retry_note`, which overrides the step-wide
    /// verdict for one task's prompt. Everything else in the context
    /// describes the *attempt*, not the feedback, and has to survive
    /// verbatim — a functional update so a field added to this struct
    /// cannot silently arrive empty at the call site.
    pub fn with_feedback(&self, feedback: String) -> Self {
        Self {
            feedback,
            ..self.clone()
        }
    }
}

/// The retry context a freshly started driver should carry, given the one
/// persisted for its feature.
///
/// Without this every restart — crash, quit, a restart while parked —
/// resumes the loop's current step with no context: [`classify`] answers
/// Greenfield, so a producer in a rework cycle re-emits its whole
/// decomposition and the stale-plan guards are off.
///
/// Dropped, rather than restored, whenever the row cannot be trusted to
/// describe a loop that is still open:
///
/// * a `failing_step_id` the current graph lacks — the feature moved to a
///   workflow version that no longer has that node, or it is blank: the
///   "clear after the next completed step" shape, which a restart cannot
///   tell whether it already cleared;
/// * the failing step's row is `completed` — the loop closed and the
///   clear that should have followed was lost.
///
/// `iteration` takes the larger of the row's and the failing step's
/// `iteration_count`, since the step row is written first and is the
/// budget's own counter, and `max` is raised to meet it — a gate's context
/// carries `1 of 1`, and a prompt must never read "attempt 3 of 1". Neither
/// is written back: the budget stays with `step_executions.iteration_count`,
/// so a resumed attempt is not counted twice.
pub fn restore_retry_context(
    persisted: Option<RetryContext>,
    step_rows: &[StepExecution],
    graph: &WorkflowGraph,
) -> Option<RetryContext> {
    let mut ctx = persisted?;
    let failing = StepId::from(ctx.failing_step_id.trim().to_string());
    if !graph.contains(&failing) {
        return None;
    }
    let row = step_rows.iter().find(|r| r.step_id == failing);
    if row.is_some_and(|r| r.status == "completed") {
        return None;
    }
    if let Some(r) = row {
        ctx.iteration = ctx.iteration.max(r.iteration_count);
    }
    ctx.max = ctx.max.max(ctx.iteration);
    Some(ctx)
}

/// The part of a retry context this decision reads.
///
/// Deliberately not the whole [`RetryContext`]: that carries
/// prompt-rendering fields (feedback prose, attempt counters) this
/// decision must not consult. One field decides it, and naming exactly
/// that keeps the rule honest.
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

/// Brackets a consumer's rejection inside a carried verdict, so the next
/// carry can find and drop it. Every carry happens after the producer has
/// re-run since the last rejection, so an older one is always answered;
/// kept, it would sit in every later producer prompt until the origin
/// completes. Plain text because it is rendered into the prompt verbatim.
const REJECTION_OPEN: &str = "[task list sent back] ";
const REJECTION_CLOSE: &str = "\n[end of task list sent back]\n\n";

fn without_rejection(feedback: &str) -> String {
    let Some(start) = feedback.find(REJECTION_OPEN) else {
        return feedback.to_string();
    };
    match feedback[start..].find(REJECTION_CLOSE) {
        Some(len) => format!(
            "{}{}",
            &feedback[..start],
            &feedback[start + len + REJECTION_CLOSE.len()..]
        ),
        None => feedback.to_string(),
    }
}

/// `in_flight`, when it names an origin a redirect can keep.
fn keepable(in_flight: Option<RetryContext>) -> Option<RetryContext> {
    in_flight.filter(|rc| !rc.failing_step_id.trim().is_empty())
}

/// The retry context a consumer's producer fault hands the producer it
/// redirects to. `own` is the context the fault opens by itself: the
/// consumer as origin, its reason as feedback, its own budget.
///
/// A fault raised by the consumer names the consumer as its origin, and
/// [`classify`] reads a consumer's own failure as [`ReworkMode::Revision`]
/// — so inside a rework cycle the producer would be asked for a whole
/// decomposition, having also lost the verdict it was meant to close. When
/// the consumer was in rework (`consumer_in_rework`, taken before the new
/// retry context replaces the in-flight one), the in-flight context is kept
/// whole — origin, verdict, attempt counters — with `own`'s reason in place
/// of any earlier rejection. Outside rework, or with nothing in flight,
/// `own` is the context, as before.
pub fn producer_fault_retry(
    in_flight: Option<RetryContext>,
    consumer_in_rework: bool,
    own: RetryContext,
) -> RetryContext {
    match keepable(in_flight) {
        Some(rc) if consumer_in_rework => RetryContext {
            feedback: format!(
                "{REJECTION_OPEN}{}{REJECTION_CLOSE}{}",
                own.feedback,
                without_rejection(&rc.feedback)
            ),
            ..rc
        },
        _ => own,
    }
}

/// The retry context a gate's redirect hands the step it sends the run back
/// to. `own` is the context the gate opens by itself: the gate as origin,
/// the reviewer's feedback, one attempt.
///
/// A gate sits downstream of the producer but upstream of the consumer, so
/// [`classify`] reads a failure originating at it as
/// [`ReworkMode::Revision`]. Inside a rework cycle that would have the
/// producer redo the whole decomposition and drop the verdict that opened
/// the cycle. When the gate is between a rework-opted producer and its
/// consumer (`gate_in_span`) and an origin is in flight, that context is
/// kept whole and the reviewer's feedback is prepended to its verdict —
/// once: the same note given again is not stacked. Otherwise `own` is the
/// context, as before.
pub fn gate_redirect_retry(
    in_flight: Option<RetryContext>,
    gate_in_span: bool,
    own: RetryContext,
) -> RetryContext {
    match keepable(in_flight) {
        Some(rc) if gate_in_span => {
            let verdict = without_rejection(&rc.feedback);
            let note = own.feedback.trim();
            let feedback = if verdict == note || verdict.starts_with(&format!("{note}\n\n")) {
                verdict
            } else {
                format!("{note}\n\n{verdict}")
            };
            RetryContext { feedback, ..rc }
        }
        _ => own,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/rework.rs"]
mod tests;
