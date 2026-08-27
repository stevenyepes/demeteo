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
//!
//! Refusing is the floor, not the whole answer. What makes picking a history
//! unsafe is that it drops the other one, and git's third move drops neither:
//! merging `origin/<feature>` into the local branch puts the branch back on
//! top of origin with both sides' commits still in it, which is the one
//! reconcile Demeteo may make on its own. [`classify_divergence`] decides that
//! off patch equivalence, because the counts alone cannot tell work only this
//! clone has apart from a branch origin rewrote underneath it.
//!
//! Both readings are taken against git wherever they are needed, and never read
//! back off a sync session row. A row records the divergence that stopped one
//! sync; the two branches it counted go on moving afterwards, here and in every
//! other clone, and the row is the one thing that cannot notice. That is why
//! every surface offering a reconcile re-measures instead of rendering counts it
//! already holds — and why those surfaces cite this paragraph rather than
//! restating it.

use serde::{Deserialize, Serialize};

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

/// What a sync may do about a divergence, once patch equivalence has been
/// read — the question [`FeatureUpstream::Diverged`]'s counts cannot answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceMove {
    /// Merge `origin/<feature>` into the local branch. Both sides' commits
    /// survive it, which is why this arm needs no human in it: a merge that
    /// can drop nothing has nothing for a person to decide.
    MergeOrigin,
    /// Move the local ref onto origin's. Origin already carries every change
    /// the local commits make, so no *content* is lost — the shas are, and
    /// whether those matter is a statement about intent that no read of the
    /// history can make.
    ResetOntoOrigin,
    /// Say what was measured and stop, which is [`diverged_refusal`] unchanged.
    Refuse,
}

/// Read a [`DivergenceMove`] off `git cherry origin/<feature> <feature>`,
/// against the `ahead` count the same divergence was measured at.
///
/// `git cherry` prints one line per commit on the local branch that the
/// upstream lacks: `-` when the upstream already carries that commit's patch
/// under some other sha, `+` when it does not. Two commits ahead of a branch
/// that already contains their changes is somebody's rebase; two commits ahead
/// of a branch that does not is work only this clone holds. The counts are
/// identical in both cases, and the safe move is not.
///
/// Each arm is chosen for what it cannot lose:
///
/// - every line `-`, and as many lines as there are commits ahead — origin
///   rewrote this branch (a rebase, a squash, an amend in another clone) and
///   holds all of its content already, so a reset onto origin drops none. It is
///   still [`DivergenceMove::ResetOntoOrigin`] and not a merge because those
///   local shas are the ones being abandoned, and a press rather than an
///   automatic move for the same reason.
/// - every line `+` — the two sides are disjoint and a merge keeps both. The
///   line count is not checked here: a merge loses nothing whatever it did not
///   look at.
/// - anything else — mixed, empty, unparseable, short of `ahead`, or `None` for
///   a read that could not run. Mixed is a partial rewrite, where a reset drops
///   the `+` commits and only the person who made it knows whether that was the
///   point. The rest are the same non-answer that [`reconcile`] refuses to read
///   as `Current`: `git cherry` saying nothing about a branch the counts called
///   ahead has not answered the question, and a caller that treats silence as
///   unanimity would reset a branch on the strength of a failed command.
///
/// `ahead` is what makes the first arm true rather than merely unanimous.
/// `git cherry` walks with `max_parents=1`, so it never prints a merge commit
/// at all — a branch two ahead whose second commit is a merge yields one `-`
/// line, and reading that as "every local commit is already upstream" resets
/// away a merge nothing examined. That is not a hypothetical shape here:
/// Demeteo's own sync and subtask merges are merge commits, and the tree one
/// carries after a hand resolution exists in no other commit on either side.
pub fn classify_divergence(cherry: Option<&str>, ahead: u64) -> DivergenceMove {
    let Some(cherry) = cherry else {
        return DivergenceMove::Refuse;
    };
    let (mut already_upstream, mut only_here) = (0u64, 0u64);
    for line in cherry.lines().filter(|line| !line.trim().is_empty()) {
        match patch_already_upstream(line) {
            Some(true) => already_upstream += 1,
            Some(false) => only_here += 1,
            None => return DivergenceMove::Refuse,
        }
    }
    match (already_upstream, only_here) {
        (0, 0) => DivergenceMove::Refuse,
        (seen, 0) if seen == ahead => DivergenceMove::ResetOntoOrigin,
        (_, 0) => DivergenceMove::Refuse,
        (0, _) => DivergenceMove::MergeOrigin,
        _ => DivergenceMove::Refuse,
    }
}

fn patch_already_upstream(line: &str) -> Option<bool> {
    let line = line.trim();
    let (already_upstream, sha) = line
        .strip_prefix('-')
        .map(|sha| (true, sha))
        .or_else(|| line.strip_prefix('+').map(|sha| (false, sha)))?;
    let sha = sha.trim_start();
    (!sha.is_empty() && sha.chars().all(|c| c.is_ascii_hexdigit())).then_some(already_upstream)
}

/// The half of [`DivergenceMove`] a person can press.
///
/// A type of its own rather than a validated [`DivergenceMove`], because this
/// is what arrives over the IPC: `refuse` sent as a request is not an
/// instruction to refuse anything, it is a caller asking for a move that does
/// not exist, and a type with no variant for it cannot be handed one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceReconcile {
    /// Merge `origin/<feature>` into the branch, keeping both histories.
    MergeOrigin,
    /// Move the branch onto `origin/<feature>`, abandoning the local commits.
    ResetOntoOrigin,
}

impl From<DivergenceReconcile> for DivergenceMove {
    fn from(reconcile: DivergenceReconcile) -> Self {
        match reconcile {
            DivergenceReconcile::MergeOrigin => DivergenceMove::MergeOrigin,
            DivergenceReconcile::ResetOntoOrigin => DivergenceMove::ResetOntoOrigin,
        }
    }
}

/// The branch pair a divergence was measured over, and by how far.
///
/// One parameter because the four are one fact: every refusal below names both
/// branches *and* both counts, and a count paired with the wrong branch name is
/// a sentence that reads as authoritative and is false.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DivergedBranch<'a> {
    pub feature: &'a str,
    pub base: &'a str,
    pub ahead: u64,
    pub behind: u64,
}

/// The move a sync makes in its worktree over a divergence, or the text it
/// stops with.
///
/// `chosen` is a person's answer to a previous stop, and it is weighed rather
/// than obeyed:
///
/// - [`DivergenceReconcile::MergeOrigin`] is taken whatever the measurement
///   says, `git cherry` unreadable included. A merge keeps both sides by
///   construction, so no reading of the history can make it the wrong thing to
///   have done — which is also why it is the arm a sync may take with nobody
///   watching.
/// - [`DivergenceReconcile::ResetOntoOrigin`] is taken only while patch
///   equivalence still says origin carries every change the local commits
///   make. The press is made against a pane that was rendered earlier, and one
///   commit pushed to the branch in between turns the same button into a
///   discard of work nobody has read. Re-reading is the whole reason the choice
///   is passed down here instead of being acted on where it was made.
///
/// Without a press the answers are [`classify_divergence`]'s own, which is what
/// the unattended `sync` node gets: the disjoint merge runs, and the two
/// readings that would pick a history stop.
pub fn divergence_move(
    branch: DivergedBranch<'_>,
    chosen: Option<DivergenceReconcile>,
    cherry: Option<&str>,
) -> Result<DivergenceReconcile, String> {
    match (chosen, classify_divergence(cherry, branch.ahead)) {
        (Some(DivergenceReconcile::MergeOrigin), _) => Ok(DivergenceReconcile::MergeOrigin),
        (Some(DivergenceReconcile::ResetOntoOrigin), DivergenceMove::ResetOntoOrigin) => {
            Ok(DivergenceReconcile::ResetOntoOrigin)
        }
        (Some(DivergenceReconcile::ResetOntoOrigin), _) => {
            Err(stale_reset_refusal(branch.feature, branch.ahead))
        }
        (None, DivergenceMove::MergeOrigin) => Ok(DivergenceReconcile::MergeOrigin),
        (None, DivergenceMove::ResetOntoOrigin) => Err(rewritten_refusal(
            branch.feature,
            branch.ahead,
            branch.behind,
        )),
        (None, DivergenceMove::Refuse) => Err(diverged_refusal(
            branch.feature,
            branch.base,
            branch.ahead,
            branch.behind,
        )),
    }
}

/// The stop for a reset that was true when it was offered and is not true now.
///
/// The only refusal here that describes a race rather than a branch: between
/// the pane rendering the offer and the press arriving, `origin/<feature>`
/// stopped containing every change the local commits make. Saying so is worth
/// its own sentence — the user is looking at a button they were shown, and
/// [`diverged_refusal`]'s "reconcile the branch yourself" would read as if they
/// had asked for something Demeteo never offers.
pub fn stale_reset_refusal(feature_branch: &str, ahead: u64) -> String {
    format!(
        "'{feature_branch}' was not reset onto origin/{feature_branch}: the reading that \
         would have made that safe no longer holds. origin/{feature_branch} no longer \
         carries every change this checkout's {ahead} commit(s) make, so the reset would \
         drop work and not only shas. Nothing was changed — sync again to see where the \
         branch now stands."
    )
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

/// The stop for a divergence [`classify_divergence`] read as
/// [`DivergenceMove::ResetOntoOrigin`], which is a different sentence because
/// a different thing was measured.
///
/// It is still a stop and not a reset. What patch equivalence proves is that
/// no *change* would be lost, and the commits themselves are what a person
/// keeps a branch for — a signed commit, a message somebody wrote, a sha an
/// open review is anchored to. So this says what was read and hands the move
/// back, where [`diverged_refusal`] can only say that something is wrong.
pub fn rewritten_refusal(feature_branch: &str, ahead: u64, behind: u64) -> String {
    format!(
        "'{feature_branch}' has diverged from origin: origin has {behind} commit(s) this \
         checkout does not, and this checkout has {ahead} commit(s) origin does not — and \
         every one of those {ahead} is already contained in origin's {behind} under a \
         different sha, which is what a rebase, a squash or an amend somewhere else looks \
         like from here. Resetting this checkout onto origin/{feature_branch} would \
         therefore drop no changes, only the {ahead} local commit(s) that carry them. \
         Whether that is what you meant is not something the history can answer, so the \
         sync stops here: reset onto origin, or merge origin in to keep both histories, \
         and sync again."
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
