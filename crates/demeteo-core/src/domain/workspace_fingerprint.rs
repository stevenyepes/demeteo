//! What "the workspace a step attempt started from" means, as one comparable
//! string (P1.14). The resume guard in
//! `adapters/step_executor/driver/run_loop/resume.rs` compares the value
//! recorded at node start against a fresh probe after a crash; everything
//! here decides what that probe *means*, and the adapter only runs git.
//!
//! **The feature is a ref, not a checkout.** The first scheme fingerprinted
//! `HEAD` of the project's orchestrator clone, on the premise that the clone
//! sits on the default branch for the whole run. It does not: a terminal
//! opened on a pipeline checks the feature branch out there, merge-back then
//! lands in it, and a human commits in it — and every feature on the project
//! shares that one clone. So the old value described whichever feature last
//! touched the clone, which made the guard blind to a moved feature branch
//! and able to park a feature whose own state had not changed. The tip of the
//! feature branch is what every step lands on, whatever is checked out.
//!
//! **Dirtiness is scoped to the feature's own checkouts** — wherever its
//! branch or one of its subtask branches is checked out. That catches a
//! human's uncommitted edits in a terminal and the partial writes a killed
//! agent leaves in its step worktree, and ignores the other features' work.
//!
//! **The scheme is versioned** so rows recorded under the old meaning are
//! never compared against the new one: on the first resume after an upgrade
//! they would all differ, and every interrupted feature would park for
//! nothing. See [`crate::domain`] for why this is synchronous.

use crate::domain::ids::SUBTASK_BRANCH_INFIX;
use crate::domain::worktree_listing::WorktreeListing;

const SCHEME_PREFIX: &str = "v2:";

/// Paths of every checkout holding `feature_branch` or one of its subtask
/// branches, primary included.
pub fn feature_checkouts<'a>(listing: &'a WorktreeListing, feature_branch: &str) -> Vec<&'a str> {
    let subtask_prefix = format!("{feature_branch}{SUBTASK_BRANCH_INFIX}");
    listing
        .all()
        .filter(|wt| {
            wt.branch
                .as_deref()
                .is_some_and(|b| b == feature_branch || b.starts_with(&subtask_prefix))
        })
        .map(|wt| wt.path.as_str())
        .collect()
}

/// `v2:<feature tip>:<clean|dirty>`, or `None` when `tip` is not an object
/// id — a probe that ran against a broken repo reads as unknown, and unknown
/// never blocks a run.
pub fn render(tip: &str, dirty: bool) -> Option<String> {
    let tip = tip.trim();
    let is_oid = matches!(tip.len(), 40 | 64) && tip.chars().all(|c| c.is_ascii_hexdigit());
    is_oid.then(|| {
        format!(
            "{SCHEME_PREFIX}{tip}:{}",
            if dirty { "dirty" } else { "clean" }
        )
    })
}

/// Whether a recorded fingerprint means the same thing [`render`] does, and
/// so may be compared against a fresh probe at all.
pub fn is_comparable(recorded: &str) -> bool {
    recorded.starts_with(SCHEME_PREFIX)
}

#[cfg(test)]
#[path = "../../tests/domain/workspace_fingerprint.rs"]
mod tests;
