//! Which branch a run is measured against. See [`crate::domain`].
//!
//! Four call sites answer this independently — the review diff's fork point,
//! the finalize step's branch summary, the harness baseline's `base_sha`, and
//! the "view diff" deep link — and before this module each of them read
//! `ProjectSettings.worktree_strategy.default_branch` directly. That is one
//! answer while every run starts at the default branch; the moment a run
//! declares its own base (`features.diff_base_branch`, V41) the four have to
//! agree, and four independent `settings.worktree_strategy` reads cannot be
//! made to.
//!
//! The squash is deliberately not among them: it parents onto the commit the
//! branch was cut from, which is [`FeatureOrigin::squash_base`] and, for a run
//! launched on a pull request, a different revision from the one resolved here.
//!
//! ## Why the declared base outranks the origin here
//!
//! [`FeatureOrigin::base_branch`] subordinates the declared base to the arm:
//! [`FeatureOrigin::DefaultBranch`] answers `None` whatever it is handed,
//! because that arm's *cut* is defined in terms of the default branch and a
//! review base must not move a start point. This function answers a different
//! question — where the diff **starts**, not where the branch was cut — so the
//! declared base wins outright, and the origin only supplies a base for a run
//! that named one at cut time and nothing since.

use crate::domain::feature_origin::FeatureOrigin;

/// The branch this run is measured against: the left side of the range each of
/// the four call sites above computes.
///
/// `None` means nothing named a branch at all — no declared base, an origin
/// with no base of its own, and a project whose default branch is unset. The
/// callers treat that as "no range", never as a default guess: a base branch
/// guessed by name yields either an empty diff or the whole repository, and
/// both read like a finished review.
pub fn resolve<'a>(
    diff_base_branch: Option<&'a str>,
    origin: &'a FeatureOrigin,
    default_branch: &'a str,
) -> Option<&'a str> {
    named(diff_base_branch)
        .or_else(|| named(origin.base_branch(None)))
        .or_else(|| named(Some(default_branch)))
}

fn named(branch: Option<&str>) -> Option<&str> {
    branch.map(str::trim).filter(|b| !b.is_empty())
}

#[cfg(test)]
#[path = "../../tests/domain/diff_base.rs"]
mod tests;
