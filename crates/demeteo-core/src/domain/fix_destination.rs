//! Where a fix run's commits land, given the request it was launched from.
//! See [`crate::domain`].
//!
//! A review run ends in findings; a fix run ends in commits, and those commits
//! have to arrive somewhere a human can merge them. Two destinations exist in
//! principle — a pull request of the fix run's own, based on the reviewed one,
//! and a commit appended to the reviewed request's own branch. **Only
//! [`FixDestination::StackedPr`] is built.** The same-branch destination is
//! deferred, not undecided: it needs a push seam that exists in neither crate
//! today, credential-helper routing for the hosts where a helper can wedge a
//! non-interactive push, and an authenticated push against a real remote before
//! any of it can be believed. Half of that is worse than none — it puts a run
//! one silent failure away from writing into somebody else's repository.
//!
//! The enum is therefore left with one variant, no `#[non_exhaustive]`, and no
//! placeholder: when the second destination lands, the compiler is what walks
//! the tree and points at every site that has to choose between them. A `_ =>`
//! arm written in advance is that choice made now, by nobody, at a call site
//! that does not exist yet.
//!
//! ## Which of the two rules wins
//!
//! [`FeatureOrigin::publish_target`](crate::domain::feature_origin::FeatureOrigin::publish_target)
//! answers the same question for every run, and states it categorically: a run
//! launched to fix a pull request opens against the branch that request merges
//! into. That is this module's *fallback*, not its answer — it is right for a
//! fork, and needlessly conservative for a request whose head branch the
//! provider already placed upstream.
//!
//! This module supersedes it for review-launched runs, and does so by feeding
//! it: `resolve` produces the base, the launcher carries it as
//! [`PublishOptions::target_branch`](crate::domain::models::PublishOptions),
//! and `publish_target` ranks that caller answer above the origin exactly so
//! this decision can reach it. Nothing here reimplements the publish path, and
//! nothing in `publish_target` needs to know a fix run from any other — a call
//! site that resolves neither still lands on the request's target branch,
//! which is the safe half of both rules.
//!
//! ## The fork case
//!
//! [`FixDestination::StackedPr`] names a base branch, and the provider resolves
//! that name in the *upstream* repository — the one this clone pushes to.
//! [`MrSummary`] carries two branch names and only one of them is guaranteed to
//! be there:
//!
//! - `target_branch` is what the reviewed request already targets, so the
//!   provider resolved it upstream when that request was opened.
//! - `source_branch` names a branch in the contributor's repository. For a fork
//!   it is not upstream at all, and a pull request naming it as a base is
//!   refused at publish time with a provider error about an unknown ref —
//!   after the run has done the work.
//!
//! So a fork request stacks on `target_branch`. What that costs is also why the
//! head branch is preferred wherever it is reachable: the fix run's worktree
//! starts from the reviewed head, so a request based on `target_branch` carries
//! the contributor's commits and the fix together, and merging it merges both.
//!
//! [`MrSummary::maintainer_can_modify`] does not rescue that case. It grants
//! writing to the branch inside the fork, which is the deferred destination; it
//! never puts that branch upstream, so nothing opened on origin can take it as
//! a base.
//!
//! ## Why an unstated permission falls back too
//!
//! Stacking on the reviewed head branch is the more permissive of the two
//! bases: merging that stack lands its commits on the contributor's branch and
//! changes what the reviewed request contains. That is a write into work
//! somebody else owns, and [`MrSummary::head_repo_push`] is the only thing that
//! says we hold it. A provider omits the field for an unauthenticated read, a
//! token without the scope, or an API too old to report it — each of which
//! means *we do not know*, which [`crate::domain::mr_summary`] already refuses
//! to spend as a yes. Unknown takes `target_branch`, which changes nothing
//! anyone else owns until a human merges it.
//!
//! One consequence is worth stating rather than leaving to be discovered: a
//! merge request carries no push permission for GitLab to report, so **no**
//! GitLab request stacks on its head branch, fork or not. That is the honest
//! reading of what the provider says, and the fallback still produces a
//! mergeable request — but it is a whole provider missing the behaviour, so it
//! is written up in `docs/KNOWN_ISSUES.md` too, where a user hunting for it
//! will look. Closing it means a real GitLab signal in `MrSummary::from_gitlab`
//! and not a provider special case here, which has no provider to ask.

use crate::domain::mr_summary::MrSummary;

/// Where a fix run publishes its commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FixDestination {
    /// A pull request of the run's own, opened against `base`, for the user to
    /// merge. `base` always names a branch in the upstream repository.
    StackedPr { base: String },
}

/// Decide where a fix run launched from `reviewed` should publish.
pub fn resolve(reviewed: &MrSummary) -> FixDestination {
    let base = if reviewed.from_fork || !reviewed.head_repo_push {
        &reviewed.target_branch
    } else {
        &reviewed.source_branch
    };
    FixDestination::StackedPr { base: base.clone() }
}

#[cfg(test)]
#[path = "../../tests/domain/fix_destination.rs"]
mod tests;
