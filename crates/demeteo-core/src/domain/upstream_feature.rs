//! Whether the local feature branch is still the whole of that branch. See
//! [`crate::domain`].
//!
//! A sync merges `origin/<base>` into `refs/heads/<feature>` **as this clone
//! holds it**, and until now nothing ever refreshed that ref: Demeteo writes
//! it, so it is only ever as current as the last thing Demeteo did with it.
//! A commit pushed to the branch from anywhere else — a person fixing a build
//! in their own clone, a suggestion committed from the pull request — is simply
//! absent, and the merge then writes a commit whose tree is the branch
//! *without* that work.
//!
//! Nothing about the result reads as a revert. Git merged the two sides it was
//! handed and merged them cleanly, the pull request shows one ordinary merge
//! commit, and the reverted fix comes back as a fresh failure of whatever it
//! had fixed — one the next agent then fixes again, from the same missing
//! commit, for as long as the branch keeps being synced. That is the failure
//! this module exists to make impossible, so the refusal below is deliberately
//! a refusal: a sync that cannot first put the local branch back on top of
//! origin has nothing safe to merge into.

use crate::ports::worktree_ops::BranchDivergence;

/// What a sync must do about `origin/<feature>` before it may merge a base.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureUpstream {
    /// Origin holds nothing this branch does not already have — the branch is
    /// level with origin, or carries local commits on top of it. Both are
    /// ordinary: a run that has not published its work yet is the second one.
    Current,
    /// Origin holds commits this branch does not, and this branch holds none
    /// origin lacks, so the local ref can simply be moved onto origin's.
    FastForward,
    /// Both sides carry commits the other does not. No fast-forward exists and
    /// this is not Demeteo's merge to make: the two histories are a person's
    /// intent about their own branch, and picking one is how the work on the
    /// other side goes missing a second way.
    Diverged { ahead: u64, behind: u64 },
}

/// Read [`FeatureUpstream`] off a divergence measured between
/// `refs/heads/<feature>` and `origin/<feature>`.
///
/// Only a *measured* zero is [`FeatureUpstream::Current`], matching the same
/// rule at the base-branch short-circuit in `sync_feature_with_upstream`: a
/// `rev-list` that did not answer says nothing about whether origin moved, and
/// reading it as "nothing to do" is exactly the silent skip this module is
/// about. An unmeasured branch is sent to the fast-forward instead, which is
/// safe to attempt in every case — `git merge --ff-only` is a no-op against an
/// ancestor and refuses anything else — so the ff is the second, independent
/// reading of the same question rather than a guess at it.
pub fn reconcile(divergence: BranchDivergence) -> FeatureUpstream {
    match (divergence.behind, divergence.ahead) {
        (Some(0), _) => FeatureUpstream::Current,
        (Some(behind), Some(ahead)) if ahead > 0 => FeatureUpstream::Diverged { ahead, behind },
        _ => FeatureUpstream::FastForward,
    }
}

/// The refusal for a divergence that was counted, before any tree exists.
pub fn diverged_refusal(
    feature_branch: &str,
    base_branch: &str,
    ahead: u64,
    behind: u64,
) -> String {
    format!(
        "'{feature_branch}' has diverged from origin: origin has {behind} commit(s) this \
         checkout does not, and this checkout has {ahead} commit(s) origin does not. \
         Merging origin/{base_branch} now would write a merge commit that silently drops \
         the {behind} on origin. {RECONCILE}"
    )
}

/// The same refusal for a divergence learned from `git merge --ff-only`
/// instead of from a count — the reading that stands when `rev-list` could not
/// answer, and the one that carries git's own words.
pub fn unmergeable_refusal(feature_branch: &str, base_branch: &str, git_error: &str) -> String {
    format!(
        "'{feature_branch}' could not be fast-forwarded to origin/{feature_branch}, so the \
         merge of origin/{base_branch} was not attempted — it would have been written on \
         top of a branch that is missing what origin has. {RECONCILE}\n\n{git_error}"
    )
}

/// The user's move, which is the same one whichever way the divergence was
/// learned: both refusals stop at exactly the point where only a person knows
/// which history is the one they meant.
const RECONCILE: &str = "Reconcile the branch yourself — push the local commits, or reset onto \
                         origin — and sync again.";

#[cfg(test)]
#[path = "../../tests/domain/upstream_feature.rs"]
mod tests;
