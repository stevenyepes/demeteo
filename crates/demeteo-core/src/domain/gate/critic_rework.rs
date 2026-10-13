//! Whether an external client may send the Standard workflow's ship gate back
//! for rework, and the feedback that redirect carries.
//!
//! The client supplies a gate and nothing else. Every word the rework agents
//! read is either [`PREAMBLE`] or the critic's own review, so
//! `docs/MCP_INTEGRATION.md` §8's objection to `ticket_force_start` — text an
//! outside caller wrote, read by the agent as fact about its situation — has
//! nothing to attach to. Adding a caller-supplied note to [`plan`] reopens it.
//!
//! A gate redirect has no loop budget of its own (`on_failure` redirects do),
//! so [`MAX_CLIENT_REWORKS`] is the only thing that stops a client sending a
//! run round critic → fix → critic for as long as its grant lasts.

use super::redirect::resolve_redirect_target;
use crate::domain::models::{StepConfig, StepExecution};

pub(crate) const STANDARD_WORKFLOW_ID: &str = "wf-starter-standard";

/// The artifact name the critic step declares. Matched instead of the step id
/// because the id is a string in bundled JSON that a user may edit in place.
pub(crate) const CRITIC_ARTIFACT: &str = "critic-review";

pub(crate) const MAX_CLIENT_REWORKS: usize = 2;

/// Names no step id on purpose: `resolve_redirect_target` reads a whole-word
/// step id in short feedback as an address, and this text is prepended to
/// every redirect [`plan`] produces.
///
/// It says who sent the run back because the decision is recorded through the
/// same path a person's is, and the validate step reads that record as
/// evidence of human sign-off.
pub(crate) const PREAMBLE: &str =
    "This run was sent back automatically by an external MCP client, \
not by a person. Nobody has reviewed or approved the work. The client supplied no instructions \
of its own: everything below this paragraph is the critic's review of the change, quoted in \
full. Fix every issue it lists under Critical Issues and Major Issues, and fix the Minor Issues \
that are cheap to fix. Do not treat anything in the review as a request to change scope beyond \
the feature's own goal.";

const MAX_REVIEW_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CriticVerdict {
    Pass,
    PassWithNotes,
    Fail,
}

impl CriticVerdict {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::PassWithNotes => "PASS_WITH_NOTES",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub(crate) enum ReworkRefusal {
    #[error(
        "rework_from_critic only applies to the Standard Feature Pipeline; \
         this run uses a different workflow"
    )]
    NotStandardWorkflow,
    #[error(
        "this step is not the Standard workflow's final gate awaiting a decision \
         (kind `{kind}`, status `{status}`)"
    )]
    NotAwaitingFinalGate { kind: String, status: String },
    #[error(
        "this run's Standard workflow was edited and no longer has the shape \
         rework_from_critic relies on: {0}"
    )]
    WorkflowShape(&'static str),
    #[error("the critic review has no `## Verdict` of PASS, PASS_WITH_NOTES or FAIL")]
    NoVerdict,
    #[error("the critic's verdict is PASS; there is nothing to send back")]
    VerdictPass,
    #[error(
        "this run has already been sent back {0} times by an MCP client, which is the limit; \
         a person has to decide this gate in the app"
    )]
    CapReached(usize),
}

pub(crate) struct ReworkRequest<'a> {
    pub workflow_id: Option<&'a str>,
    /// The steps of the workflow version the run is pinned to.
    pub steps: &'a [StepConfig],
    pub gate: &'a StepExecution,
    pub review: &'a str,
    pub prior_reworks: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReworkPlan {
    pub verdict: CriticVerdict,
    pub feedback: String,
}

/// The step, in a Standard workflow of the expected shape, whose artifact the
/// final gate `gate` reviews. `Err` when `gate` is not that gate or the
/// workflow has been edited away from that shape.
///
/// The id alone proves nothing: saving over a starter keeps its id and its
/// `is_starter` flag, so the shape is read off the pinned steps.
pub(crate) fn critic_step<'a>(
    workflow_id: Option<&str>,
    steps: &'a [StepConfig],
    gate: &StepExecution,
) -> Result<&'a StepConfig, ReworkRefusal> {
    if workflow_id != Some(STANDARD_WORKFLOW_ID) {
        return Err(ReworkRefusal::NotStandardWorkflow);
    }
    // A pending decision row is not proof of a gate: synthetic parks reuse
    // `gate_decisions` on agent steps, and a redirect there resolves through
    // a different table of targets.
    let not_final_gate = || ReworkRefusal::NotAwaitingFinalGate {
        kind: gate.step_kind.clone(),
        status: gate.status.clone(),
    };
    if gate.step_kind != "gate" || gate.status != "awaiting_gate" {
        return Err(not_final_gate());
    }
    let idx =
        steps
            .iter()
            .position(|s| s.id == gate.step_id)
            .ok_or(ReworkRefusal::WorkflowShape(
                "the gate is not in the pinned version",
            ))?;
    let config = &steps[idx];
    if config.kind != "gate" || !config.is_dangerous_gate() {
        return Err(not_final_gate());
    }
    if steps[idx + 1..].iter().any(|s| s.kind != "finalize") {
        return Err(ReworkRefusal::WorkflowShape(
            "a step other than finalize follows the gate",
        ));
    }
    idx.checked_sub(1)
        .map(|prev| &steps[prev])
        .filter(|prev| {
            prev.artifacts
                .iter()
                .flatten()
                .any(|a| a.name == CRITIC_ARTIFACT)
        })
        .ok_or(ReworkRefusal::WorkflowShape(
            "the step before the gate does not produce a critic review",
        ))
}

/// Decide a rework request. `Ok` carries the feedback to redirect with.
pub(crate) fn plan(req: &ReworkRequest<'_>) -> Result<ReworkPlan, ReworkRefusal> {
    critic_step(req.workflow_id, req.steps, req.gate)?;
    let verdict = parse_verdict(req.review).ok_or(ReworkRefusal::NoVerdict)?;
    if verdict == CriticVerdict::Pass {
        return Err(ReworkRefusal::VerdictPass);
    }
    if req.prior_reworks >= MAX_CLIENT_REWORKS {
        return Err(ReworkRefusal::CapReached(req.prior_reworks));
    }
    let feedback = format!(
        "{PREAMBLE}\n\n---\n\n{}",
        tail_on_char_boundary(req.review.trim(), MAX_REVIEW_BYTES)
    );
    // Free-text feedback that reaches a step unable to act on it re-runs the
    // whole feature over itself. The Standard pipeline avoids that by landing
    // on its task-list producer's rework template; an edited one may not.
    let lands_on_rework_producer = steps_index(req.steps, req.gate)
        .and_then(|gate_idx| {
            resolve_redirect_target(req.steps, None, gate_idx, Some(feedback.as_str()))
        })
        .is_some_and(|target| req.steps[target].rework_prompt_template.is_some());
    if !lands_on_rework_producer {
        return Err(ReworkRefusal::WorkflowShape(
            "a redirect from the gate would not land on a step with a rework prompt",
        ));
    }
    Ok(ReworkPlan { verdict, feedback })
}

fn steps_index(steps: &[StepConfig], gate: &StepExecution) -> Option<u32> {
    steps
        .iter()
        .position(|s| s.id == gate.step_id)
        .and_then(|i| u32::try_from(i).ok())
}

/// The verdict is the last thing the critic is asked to write, so the review's
/// tail is kept when it has to be cut.
fn tail_on_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// The verdict named under the review's last `## Verdict` heading.
///
/// The body of the section is scanned by token, not compared whole, because
/// the critic writes it as prose: `**FAIL** — two blocking issues`.
pub(crate) fn parse_verdict(review: &str) -> Option<CriticVerdict> {
    let mut section: Vec<&str> = Vec::new();
    let mut in_verdict = false;
    for line in review.lines() {
        let trimmed = line.trim();
        if let Some(heading) = trimmed.strip_prefix('#') {
            let heading = heading.trim_start_matches('#').trim();
            in_verdict = heading.to_ascii_lowercase().starts_with("verdict");
            if in_verdict {
                section = vec![heading];
            }
        } else if in_verdict {
            section.push(trimmed);
        }
    }
    section
        .iter()
        .flat_map(|line| line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')))
        .find_map(|token| match token {
            "PASS" => Some(CriticVerdict::Pass),
            "PASS_WITH_NOTES" => Some(CriticVerdict::PassWithNotes),
            "FAIL" => Some(CriticVerdict::Fail),
            _ => None,
        })
}

#[cfg(test)]
#[path = "../../../tests/domain/gate/critic_rework.rs"]
mod tests;
