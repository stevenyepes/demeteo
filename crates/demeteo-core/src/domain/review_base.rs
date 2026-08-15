//! Where a reviewing step's diff starts.
//!
//! A step that judges work does not need the work *shipped to it* — every step
//! worktree is cut from the feature branch tip, so the code and its whole
//! history are already on disk where the agent is standing. What the agent
//! cannot recover on its own is the **left side of the range**: the commit
//! where the branch last shared history with the base this run declared itself
//! measured against ([`diff_base::resolve`](crate::domain::diff_base::resolve)).
//! `HEAD~1` is one ticket, that base is the project's default branch only for a
//! run that started there and may be called anything, and `origin/` may not
//! exist at all. So the orchestrator, which resolves that commit already for
//! its own diff, says it here instead of leaving the agent to guess.
//!
//! ## Why this is a prose block and not a `{{base_ref}}` scalar
//!
//! A scalar looks friendlier — `git diff {{base_ref}}..HEAD` reads well in a
//! template — but it has no safe empty rendering. `PromptContext::render`
//! collapses an unresolved token to `""`, and `git diff ..HEAD` is not an
//! error: git reads the missing left side as `HEAD`, so the command succeeds
//! and returns nothing. A reviewer handed an empty diff concludes the branch
//! is empty, which is the "absent is not green" failure
//! [`render_executable`](crate::domain::prompt_context::PromptContext::render_executable)
//! exists to prevent, arriving through the prompt instead of the shell. The
//! block below has no such rendering: when the fork point is unknown it says
//! so and gives a different procedure.
//!
//! Placement mirrors [`platform_context`](crate::domain::platform_context) —
//! that module's header carries the argument for why a block a template does
//! not name still has to reach the agent, and it applies here unchanged.
//! Where the two differ is the fallback: the platform block is unconditional,
//! this one auto-places only for the capabilities that exist to judge work
//! ([`StepCapability::Verify`], [`StepCapability::ReadOnly`]). An implement or
//! spec step gets it only by naming the token, because paying ~700 bytes a turn
//! to tell an author where the diff starts is worth it exactly when the step's
//! job is to read the diff.

use crate::domain::permission::StepCapability;

/// The `{{review_base_section}}` token a template may use to place the block
/// itself.
const TOKEN: &str = "{{review_base_section}}";

/// Where one render puts the block: what the token binds to, and what still has
/// to go in front of the rendered prompt. At most one is ever non-empty.
pub(crate) struct ReviewBasePlacement {
    /// What `{{review_base_section}}` renders as.
    pub(crate) bound: String,
    /// What goes in front of the rendered prompt — empty when the template
    /// asked for the block by name, or when this capability does not review.
    pub(crate) prefix: String,
}

/// Whether this step wants the block at all.
///
/// Callers gate the `git merge-base` round trip on this: a step that will not
/// render the block should not pay two DB reads and a git call per attempt to
/// resolve a commit nobody reads. Same reasoning as `needs_harness_briefing`.
pub(crate) fn needs_review_base(capability: StepCapability, template: &str) -> bool {
    template.contains(TOKEN) || reviews_by_capability(capability)
}

/// The capabilities whose whole purpose is judging work somebody else did.
/// Both are barred from writing source, so a diff range is the only thing they
/// can act on.
fn reviews_by_capability(capability: StepCapability) -> bool {
    matches!(
        capability,
        StepCapability::Verify | StepCapability::ReadOnly
    )
}

/// Decide the placement.
///
/// `fork_point` is `None` when nothing named a base for this run at all or
/// `git merge-base` could not answer. That case is rendered, not suppressed:
/// the agent is going to run *some* git command either way, and the difference
/// between "here is the range" and "work it out from the log" is the difference
/// between a review and a guess.
pub(crate) fn place_review_base(
    fork_point: Option<&str>,
    branch: &str,
    capability: StepCapability,
    template: &str,
) -> ReviewBasePlacement {
    if !needs_review_base(capability, template) {
        return ReviewBasePlacement {
            bound: String::new(),
            prefix: String::new(),
        };
    }
    let section = review_base_section(fork_point, branch);
    if template.contains(TOKEN) {
        ReviewBasePlacement {
            bound: section,
            prefix: String::new(),
        }
    } else {
        ReviewBasePlacement {
            bound: String::new(),
            prefix: section,
        }
    }
}

/// The block itself. Self-contained, carrying its own trailing rule, so the
/// bytes are identical whether a template placed it or the prefix did.
fn review_base_section(fork_point: Option<&str>, branch: &str) -> String {
    match fork_point {
        Some(sha) => format!(
            "## Reviewing this feature's change\n\
             \n\
             This feature's work begins at `{sha}` — the commit where `{branch}` last \
             shared history with the branch this run is measured against. Everything after \
             it on this branch is the change under review, including work committed by \
             earlier steps of this run.\n\
             \n\
             ```\n\
             git diff --name-status {sha}..HEAD    # what changed\n\
             git diff --stat {sha}..HEAD           # how much\n\
             git log --oneline {sha}..HEAD         # in what order\n\
             git diff {sha}..HEAD -- <path>        # one file, in full\n\
             ```\n\
             \n\
             A bare `git diff` is normally empty here, and that is not a sign that nothing \
             happened: the implementation is committed, not left in the working tree. Use \
             the range above, and do not substitute a base branch you guessed by name — \
             the branch this run is measured against is not always the one this project \
             defaults to, and it may be called neither `main` nor `master`.\n\
             \n\
             ---\n\
             \n"
        ),
        None => format!(
            "## Reviewing this feature's change\n\
             \n\
             The orchestrator could not determine where `{branch}` diverged from the branch \
             this run is measured against, so it cannot hand you an exact range. Orient \
             yourself from the log instead:\n\
             \n\
             ```\n\
             git log --oneline --decorate -n 30\n\
             ```\n\
             \n\
             This feature's commits are at the top; the change begins at the first commit \
             below them that is not one of them. Do not guess a base branch by name — a \
             wrong base silently yields either an empty review or one covering the whole \
             repository, and both read like a finished review.\n\
             \n\
             ---\n\
             \n"
        ),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/review_base.rs"]
mod tests;
