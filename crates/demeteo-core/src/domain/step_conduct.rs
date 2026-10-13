//! How a step is told to work its turn — the engine-owned preamble beside
//! [`step_boundary`](crate::domain::step_boundary).
//!
//! The boundary says what a capability must not do; this says how any turn is
//! conducted inside it. Three things current models get wrong in an unattended
//! pipeline, and that no workflow template can be relied on to say for itself:
//!
//! - **Stopping short.** A turn that ends on "I'll now write the file" has
//!   written nothing, and the orchestrator reads that as a declared artifact
//!   never produced — a retry at full cost for a sentence.
//! - **Reporting intent as fact.** The validate step already has to separate a
//!   ticket's self-reported lines from the commands Demeteo observed it run;
//!   asking the agent to audit its own claims first is the cheaper half of that
//!   defence.
//! - **Unrequested scope.** At the default effort an implement turn tidies
//!   nearby code and commits scratch checks as permanent tests. The rework
//!   loop then judges changes nobody asked for.
//!
//! Unconditional for the same reason `place_platform_context` is: a template
//! stored in a user's database cannot know about a block that did not exist
//! when it was written.

use crate::domain::permission::StepCapability;

/// The heading the block renders under. Tests and readers key on it.
pub(crate) const CONDUCT_HEADING: &str = "## How to work this turn";

const AUTONOMY: &str = "Nobody is watching this turn and nobody can answer a question mid-task, \
so proceed on reversible actions that follow from the task without asking. Before ending, \
read your last paragraph: if it is a plan, a question, or a promise about work not yet done, \
do that work now. End only when the deliverable exists or you are blocked on something only \
a human can decide.";

const GROUNDED_CLAIMS: &str = "Audit each claim in your report against a tool result from \
this session. Report only work you can point to evidence for, say plainly what you did not \
verify, and when a test fails say so with its output.";

const SCOPE: &str = "The task sets the scope. A pre-existing bug, a performance concern or \
behaviour the task does not mention is reported as a follow-up in your reply, not fixed in \
this change, unless the task cannot work without it. Where the task is ambiguous, implement \
the reading its wording and the surrounding code most directly support and state that \
assumption. Scratch scripts and quick checks need not be kept; commit tests only where the \
task asks for them or this repository already keeps tests for this kind of change, sized like \
the neighbouring test files. Edit files surgically rather than rewriting them when the result \
is the same.";

/// Prepend the conduct block to `prompt`.
///
/// The scope paragraph is for [`StepCapability::Implement`] only: the other
/// capabilities cannot touch source, so the fence already holds their scope,
/// and a paragraph about committing tests read by a step that may not write
/// tests is noise at best.
pub(crate) fn inject_turn_conduct(prompt: &str, capability: StepCapability) -> String {
    let mut block = format!("{CONDUCT_HEADING}\n\n{AUTONOMY}\n\n{GROUNDED_CLAIMS}\n");
    if capability == StepCapability::Implement {
        block.push_str(&format!("\n{SCOPE}\n"));
    }
    format!("{block}\n---\n\n{prompt}")
}

#[cfg(test)]
#[path = "../../tests/domain/step_conduct.rs"]
mod tests;
