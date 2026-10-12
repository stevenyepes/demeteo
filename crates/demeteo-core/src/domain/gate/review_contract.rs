//! What a review step is asked to write, and how a gate reads it back.
//!
//! Ask and parse live together for the reason
//! [`verifier::verdict`](crate::domain::verifier::verdict) gives: the shape the
//! prompt requests and the shape the reader accepts are one contract.
//!
//! **A step field, not a prompt convention.** A step that sets
//! [`StepConfig::review_artifact`] gets [`contract_section`] appended by the
//! engine, so a workflow somebody built on the canvas produces a readable
//! review without its author knowing the format exists.
//!
//! **The counts decide; the written verdict only has to agree.** The Standard
//! critic prompt never said what `PASS_WITH_NOTES` meant, so a review listing
//! major issues could carry it. [`standing`] derives blocking-or-not from the
//! counts and treats a `## Verdict` that says otherwise as a review nobody can
//! rely on.
//!
//! **Every unreadable case holds the gate.** This is read to let
//! [`GateAutonomy::Clean`](crate::domain::gate_autonomy::GateAutonomy::Clean)
//! approve a ship gate with nobody looking, so there is no lenient reading:
//! a missing block, a missing verdict and a disagreement all answer
//! [`ReviewStanding::Unreadable`].
//!
//! Synchronous and total, per the [`domain`](crate::domain) rule.

use serde::Deserialize;

use super::critic_rework::{parse_verdict, CriticVerdict};
use crate::domain::artifact::ArtifactCapture;
use crate::domain::json_block::find_json_block;
use crate::domain::models::StepConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReviewCounts {
    pub critical: u32,
    pub major: u32,
    pub minor: u32,
}

impl ReviewCounts {
    fn blocks(self) -> bool {
        self.critical > 0 || self.major > 0
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CountsBlock {
    review: ReviewCounts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewStanding {
    /// No critical and no major issue. Minor issues do not hold a gate.
    Clean,
    Blocking,
    Unreadable(&'static str),
}

impl ReviewStanding {
    /// Why a gate that turned on this review is being held for a person, in
    /// words shown to that person. `None` for a review that holds nothing.
    pub(crate) fn held_reason(self) -> Option<&'static str> {
        match self {
            Self::Clean => None,
            Self::Blocking => Some("the review lists critical or major issues"),
            Self::Unreadable(why) => Some(why),
        }
    }
}

/// The section appended to a review step's prompt. `path` is where the step's
/// declared review artifact is written.
pub(crate) fn contract_section(path: &str) -> String {
    format!(
        "## Review Format (orchestrator contract)\n\n\
         Write the review to `{path}` with these four sections, in this order. \
         Keep every heading even when its section is empty, and write `None.` \
         under it.\n\
         - `## Critical Issues` — must be fixed before this ships\n\
         - `## Major Issues` — should be fixed before this ships\n\
         - `## Minor Issues` — optional improvements\n\
         - `## Verdict` — `FAIL` if you listed any critical or major issue, \
         `PASS_WITH_NOTES` if you listed only minor issues, `PASS` if you \
         listed none\n\n\
         List each issue as its own bullet. End the file with one JSON object \
         counting the bullets in each section, and write nothing after it:\n\n\
         {{\"review\": {{\"critical\": 0, \"major\": 0, \"minor\": 0}}}}\n\n\
         The orchestrator reads that object, not the prose. A project may let a \
         review with no critical and no major issue approve the ship gate \
         without a person looking, so the severity you give an issue is a \
         decision about whether someone sees this change before it ships. A \
         review whose counts are missing, or disagree with its Verdict, is held \
         for a person."
    )
}

/// `prompt` with [`contract_section`] appended when `step` declares a review
/// whose artifact is written to a known path; unchanged otherwise.
pub(crate) fn append_contract(prompt: String, step: &StepConfig) -> String {
    match review_path(step) {
        Some(path) => format!("{prompt}\n\n{}\n", contract_section(path)),
        None => prompt,
    }
}

/// Where `step` writes the artifact its `review_artifact` names. `None` when
/// it names none, names one the step does not declare, or names one captured
/// some other way than by path — the contract has to tell the agent a file.
pub(crate) fn review_path(step: &StepConfig) -> Option<&str> {
    let name = step.review_artifact.as_deref()?;
    step.artifacts
        .iter()
        .flatten()
        .find(|decl| decl.name == name)
        .and_then(|decl| match &decl.capture {
            ArtifactCapture::LastWriteTo { path } => Some(path.as_str()),
            _ => None,
        })
}

/// The review step whose review covers the gate at `gate_idx`.
///
/// Only other gates may stand between the two. Anything else that ran after
/// the review could have changed what it reviewed, and a review of code that
/// has since moved is not a review of what ships.
pub(crate) fn review_step_for_gate(steps: &[StepConfig], gate_idx: usize) -> Option<&StepConfig> {
    steps
        .get(..gate_idx)?
        .iter()
        .rev()
        .find(|step| step.kind != "gate")
        .filter(|step| review_path(step).is_some())
}

pub(crate) fn standing(review: &str) -> ReviewStanding {
    let Some((_, block)) = find_json_block::<CountsBlock, _>(review, |_| true) else {
        return ReviewStanding::Unreadable("the review carries no issue counts");
    };
    let Some(written) = parse_verdict(review) else {
        return ReviewStanding::Unreadable("the review has no `## Verdict`");
    };
    let blocks = block.review.blocks();
    if blocks != (written == CriticVerdict::Fail) {
        return ReviewStanding::Unreadable("the review's Verdict disagrees with its issue counts");
    }
    if blocks {
        ReviewStanding::Blocking
    } else {
        ReviewStanding::Clean
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/gate/review_contract.rs"]
mod tests;
