//! Parking a step for a human, and what their answer means.
//!
//! Several situations reach a human mid-run with the same shape — *this step
//! cannot proceed, no retry changes that, and a person can decide*:
//!
//! - a node interrupted by a restart whose workspace moved underneath it
//!   (the resume fingerprint guard),
//! - a rework cycle whose producer emitted no tickets because it found
//!   nothing an implementation ticket could fix, and
//! - a validate verdict that only evidence is missing, or that repeats itself
//!   over unchanged code ([`crate::domain::verifier::park`]).
//!
//! Before this, only the first could park; the second returned
//! `NonRetryable` and ended the run. Its own prompt calls emitting nothing
//! *"a supported answer, not a failure: it ends the run and puts the
//! decision in front of a human"* — and ending the run put it in front of
//! nobody, leaving the reason in a database column.
//!
//! The park mechanics are shared (`adapters::step_executor::gate_park`);
//! what differs is the reason and whether `redirect` means anything, which
//! is what [`HumanPark`] carries.

use crate::domain::gate::redirect::feedback_names_step;
use crate::domain::ids::StepId;
use crate::domain::models::GateDecision;

/// A request to stop and ask a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HumanPark {
    /// What the human reads **before** deciding.
    ///
    /// The load-bearing field. A park whose reason only surfaces after the
    /// answer — which is what the resume guard did, using its mismatch
    /// string solely to build the decline message — asks someone to choose
    /// blind.
    pub reason: String,
    /// Where `redirect` sends the run, or `None` when redirect is not a
    /// meaningful answer to this park.
    ///
    /// The resume guard has no target: its only question is "safe to re-run
    /// here?", and there is no earlier step that makes a moved workspace
    /// safe. A zero-ticket rework does have one — the producer that emitted
    /// nothing can be told what to emit instead.
    pub redirect_to: Option<StepId>,
    /// Other targets a redirect may take, chosen by naming the step in the
    /// feedback — the same addressing a real gate honours
    /// ([`feedback_names_step`]). Empty for a park whose only meaningful
    /// redirect is `redirect_to`.
    pub alternatives: Vec<RedirectOption>,
    /// Prefixed to what `redirect_to` receives. The default target is the one
    /// the human did not name, so the park — not the human — has to say what
    /// it is being sent there to do.
    pub redirect_brief: Option<String>,
}

/// A step a human may name in a park's redirect, and where naming it lands.
///
/// The two differ for a `sequence` step whose task list has a rework-opted
/// producer: it cannot act on free text, so a redirect naming it lands on the
/// producer instead ([`crate::domain::gate::redirect`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedirectOption {
    pub named: StepId,
    pub lands_on: StepId,
}

impl HumanPark {
    /// A park with at most one redirect target and nothing to brief it on.
    pub fn new(reason: String, redirect_to: Option<StepId>) -> Self {
        Self {
            reason,
            redirect_to,
            alternatives: Vec::new(),
            redirect_brief: None,
        }
    }
}

/// What the run does with the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParkResolution {
    /// Approved: the step is done. For a zero-ticket rework that is
    /// literally true — there was nothing to implement.
    Complete,
    Redirect {
        target: StepId,
        feedback: String,
    },
    Fail(String),
    /// The run was cancelled while parked.
    Cancelled,
}

/// Resolve a parked step against the decision that arrived.
///
/// `None` means the waiter woke on cancellation rather than an answer.
///
/// Total and synchronous, over the two values the caller already holds, so
/// the whole table is assertable without a driver. It is shared by both
/// parks deliberately: the alternative is two divergent copies of the same
/// three-way answer, and the resume guard's rules were previously spelled
/// inline where nothing could reach them.
pub fn resolve_park(park: &HumanPark, decision: Option<&GateDecision>) -> ParkResolution {
    let Some(decision) = decision else {
        return ParkResolution::Cancelled;
    };
    match decision.decision.as_deref() {
        Some("approve") => ParkResolution::Complete,
        Some("redirect") => {
            let words = decision
                .feedback
                .clone()
                .unwrap_or_else(|| park.reason.clone());
            if let Some(named) = decision.feedback.as_deref().and_then(|f| {
                park.alternatives
                    .iter()
                    .find(|o| feedback_names_step(f, &o.named.0))
            }) {
                return ParkResolution::Redirect {
                    target: named.lands_on.clone(),
                    feedback: words,
                };
            }
            redirect_to_default(park, words)
        }
        other => ParkResolution::Fail(format!(
            "{}; user answered '{}'",
            park.reason,
            other.unwrap_or("none")
        )),
    }
}

fn redirect_to_default(park: &HumanPark, words: String) -> ParkResolution {
    match park.redirect_to.as_ref() {
        Some(target) => ParkResolution::Redirect {
            target: target.clone(),
            feedback: match park.redirect_brief.as_deref() {
                Some(brief) => format!("{}\n\n{}", brief, words),
                None => words,
            },
        },
        // A redirect offered by the shared modal but meaningless for this
        // park. Failing is the honest answer and preserves the resume
        // guard's documented behaviour; silently approving would resume a
        // node the human declined to resume.
        None => ParkResolution::Fail(format!(
            "{}; user answered 'redirect', which this park cannot honour",
            park.reason
        )),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/step_park.rs"]
mod step_park_tests;
