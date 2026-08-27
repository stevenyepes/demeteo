//! Why a sync did not land, in the two shapes the user acts on differently.
//! See [`crate::domain`].
//!
//! Most of the ways `sync_feature_with_upstream` can fail never reach a merge
//! verdict — an unreachable remote, a base branch that does not exist upstream,
//! a worktree that would not provision, a missing repository row, a merge the
//! connection cut short. Every one of them used to arrive at the UI as a merge
//! conflict, so the banner read "Merge conflict in 0 file(s)" and offered to
//! spend an agent on a tree with nothing in it to resolve. The difference
//! between the two classes is not a message, it is what the user's next move
//! is, so it is a variant.
//!
//! The class cannot be recovered downstream, which is why it is carried from
//! the return statement rather than derived at the boundary. `files` being
//! empty proves nothing: the porcelain parse answers an empty vec on any
//! transport error, and a merge that succeeded and then failed to push has an
//! empty list beside a real merge commit. `worktree_path` being absent is
//! worse — a genuine conflict whose worktree probe failed looks identical.
//! Nothing may infer the class.

use serde::{Deserialize, Serialize};

use crate::domain::models::{ConflictFile, UpstreamSyncFailure, UpstreamSyncOutcome};
use crate::domain::upstream_feature::DivergenceReconcile;
use crate::ports::step_executor::SyncOutcomeView;

/// Where a sync stopped before it could merge.
///
/// Crosses IPC inside [`SyncOutcomeView::Blocked`] as its snake_case serde
/// form, which `src/types.ts` declares as `SyncBlockedStage` to receive it;
/// renaming a variant is a frontend change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncBlockedStage {
    /// `git fetch origin -- <base>` failed: the remote is unreachable, or the
    /// stored credentials no longer open it.
    Fetch,
    /// The fetch succeeded but `origin/<base>` resolves to nothing — the run's
    /// base branch is wrong, not the network.
    BaseRefMissing,
    /// `git worktree add` for the throwaway sync worktree failed.
    WorktreeProvision,
    /// `git merge` was abandoned rather than answered — the channel dropped or
    /// the deadline expired. Whatever the worktree holds, it is not a verdict.
    Merge,
    /// The merge landed cleanly and is committed locally; publishing it to
    /// origin failed. One of the two stages where work exists and is at risk.
    Push,
    /// The merge landed cleanly, and the project's harness then answered red
    /// in the tree it produced, so it was never pushed.
    ///
    /// Not a conflict, and the distinction is the reason this stage exists:
    /// git merges text, so two edits that never touch the same line merge
    /// without complaint and can still leave a tree that does not build — a
    /// field added to a struct on the base branch against a new literal of it
    /// on the feature branch is the whole failure, in two files that have
    /// nothing to say to each other. Nothing about such a tree is *unmerged*,
    /// so there is no `MERGE_HEAD`, no conflicted path, and nothing for the
    /// conflict resolver to open.
    Verify,
    /// The local feature branch could not be put back on top of
    /// `origin/<feature>`, so there was no state to merge the base *into*
    /// ([`crate::domain::upstream_feature`]). The one stage that stops the sync
    /// over the feature branch rather than over the base.
    ///
    /// A divergence alone does not reach here: the arm that keeps both sides is
    /// taken by the sync itself
    /// ([`divergence_move`](crate::domain::upstream_feature::divergence_move)),
    /// so what is left is the readings that would pick a history and the moves
    /// git refused.
    FeatureDiverged,
    /// The feature's project repository row could not be resolved, so no git
    /// command was ever issued.
    RepoContext,
    /// The previous sync's resolution is still committed on the branch and
    /// unpublished — or the row that would rule that out could not be read — so
    /// no new sync was started
    /// ([`resync_refusal`](crate::domain::sync_session::resync_refusal)).
    HeldResolution,
    /// Another sync or resolution of this feature was already running, so this
    /// one never started. The only stage that is a refusal rather than a
    /// verdict, and the only one never written to a row: the sync it would
    /// describe belongs to the turn that holds the slot, and overwriting that
    /// turn's row is the thing the refusal exists to avoid.
    TurnInFlight,
}

impl SyncBlockedStage {
    /// The stable identifier the column holds (migration V46), which is the
    /// serde spelling above and the same one the frontend union carries.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fetch => "fetch",
            Self::BaseRefMissing => "base_ref_missing",
            Self::WorktreeProvision => "worktree_provision",
            Self::Merge => "merge",
            Self::Push => "push",
            Self::Verify => "verify",
            Self::FeatureDiverged => "feature_diverged",
            Self::RepoContext => "repo_context",
            Self::HeldResolution => "held_resolution",
            Self::TurnInFlight => "turn_in_flight",
        }
    }

    /// `None` for anything unrecognised, so a row written by a newer build is
    /// read as a blocked sync whose stage this build cannot name — which is
    /// what every pre-V46 row is too, and what the pane already has copy for.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fetch" => Some(Self::Fetch),
            "base_ref_missing" => Some(Self::BaseRefMissing),
            "worktree_provision" => Some(Self::WorktreeProvision),
            "merge" => Some(Self::Merge),
            "push" => Some(Self::Push),
            "verify" => Some(Self::Verify),
            "feature_diverged" => Some(Self::FeatureDiverged),
            "repo_context" => Some(Self::RepoContext),
            "held_resolution" => Some(Self::HeldResolution),
            "turn_in_flight" => Some(Self::TurnInFlight),
            _ => None,
        }
    }

    /// Whether a sync that stopped here stopped before it had a row of its own
    /// to say so in.
    ///
    /// The session is opened after the held-resolution read, after the turn
    /// slot is claimed and after the repository is resolved, so these stages
    /// leave whatever the *previous* sync recorded standing. A caller that
    /// answers with the row — the reconcile does, because a conflicted or
    /// blocked reconcile is a session and not a return value — would hand the
    /// user an older sync's verdict as the result of the press they just made.
    pub fn precedes_the_session(self) -> bool {
        matches!(
            self,
            Self::RepoContext | Self::TurnInFlight | Self::HeldResolution
        )
    }
}

/// Whether a failed `git merge` is a blocked sync, and at which stage.
///
/// `None` is the only answer that means unmerged paths: a non-zero exit is the
/// one shape that reached a verdict
/// ([`classify_exec_failure`](crate::domain::harness_failure::classify_exec_failure)).
/// A merge the transport cut short or the deadline abandoned leaves a worktree
/// nobody can describe — possibly clean, possibly half-applied — and reading it
/// as a conflict is what sends the resolver looking for a `MERGE_HEAD` that was
/// never written.
pub fn merge_failure_stage(err: &str) -> Option<SyncBlockedStage> {
    use crate::domain::harness_failure::HarnessExecFailure;

    match crate::domain::harness_failure::classify_exec_failure(err) {
        HarnessExecFailure::Transport | HarnessExecFailure::Timeout => {
            Some(SyncBlockedStage::Merge)
        }
        HarnessExecFailure::NonZeroExit => None,
    }
}

/// [`merge_failure_stage`] for the reconcile that runs before the base merge,
/// where one of the two moves cannot have left a conflict.
///
/// [`DivergenceReconcile::ResetOntoOrigin`] has no second side to be unmerged
/// with, so the one thing it cannot have left is a conflict: what it refused
/// over is an edit in the tree, and a resolver sent in there would find nothing
/// to resolve. Transport and timeout keep their own stage, on the terms this
/// module sets.
///
/// [`DivergenceReconcile::MergeOrigin`] is the base merge's own split, and
/// deliberately not a second one: everything downstream reads the tree, and the
/// tree a conflicted reconcile leaves has the same shape. What differs is which
/// ref came in, and only `MERGE_HEAD` says.
pub fn reconcile_failure_stage(move_: DivergenceReconcile, err: &str) -> Option<SyncBlockedStage> {
    match move_ {
        DivergenceReconcile::MergeOrigin => merge_failure_stage(err),
        DivergenceReconcile::ResetOntoOrigin => {
            merge_failure_stage(err).or(Some(SyncBlockedStage::FeatureDiverged))
        }
    }
}

/// Whether a red harness over a merged tree withholds it, and at which stage.
///
/// Both halves of a sync ask this one function, and it is one function on
/// purpose: a clean merge and a resolved conflict reach the same pull request,
/// so a project whose two halves gated on different terms would be answering
/// one question twice. Only what each half withholds differs — the clean half
/// a push from a merge already committed, the conflicted half the commit
/// itself — and that belongs to the callers, not here.
///
/// [`merge_failure_stage`]'s question, asked one stage later and answered by
/// the same rule: only a non-zero exit is a verdict. A harness the transport
/// cut short or the deadline abandoned never ran, and a build that never ran
/// is not a red build. The two mistakes are not the same size. Letting an
/// unverified merge through leaves the pull request exactly where a sync
/// without this gate would have left it; withholding one on a dropped
/// connection strands a real merge locally and tells the user their branch is
/// broken on the strength of nothing.
///
/// `None` therefore means "let it through", for both of the reasons a caller
/// can have: the harness passed, or nobody is in a position to say it did
/// not.
pub fn verify_failure_stage(err: &str) -> Option<SyncBlockedStage> {
    use crate::domain::harness_failure::HarnessExecFailure;

    match crate::domain::harness_failure::classify_exec_failure(err) {
        HarnessExecFailure::NonZeroExit => Some(SyncBlockedStage::Verify),
        HarnessExecFailure::Transport | HarnessExecFailure::Timeout => None,
    }
}

/// What the workflow `sync` step does next about a failure.
///
/// A blocked sync must not reach the resolver: the resolution turn opens by
/// probing for `MERGE_HEAD`, and its absence is reported as "No active merge
/// in progress. Please run 'Sync with main' first." — which replaces the real
/// cause with an instruction to redo the thing that just failed.
#[derive(Debug, PartialEq)]
pub enum SyncStepNext<'a> {
    Resolve {
        files: &'a [ConflictFile],
        worktree_path: Option<&'a str>,
        /// Whether the node may report the sync done once this is resolved.
        ///
        /// `false` is the reconcile's own conflict, which stops the sync before
        /// the base merge is even attempted. A resolution puts
        /// `origin/<feature>` on the branch and nothing else, so a node that
        /// completed on it would be signalling a synced branch that never
        /// received `origin/<base>` — and the run continues onto a stale base
        /// with no drift signal left to stop it. The pane's answer to this is a
        /// second press of Sync; a workflow has nobody to press it.
        resolves_the_base_merge: bool,
    },
    Fail(&'a str),
}

pub fn step_next(failure: &UpstreamSyncFailure) -> SyncStepNext<'_> {
    match failure {
        UpstreamSyncFailure::Conflict {
            report,
            worktree_path,
            resolves_the_base_merge,
        } => SyncStepNext::Resolve {
            files: &report.files,
            worktree_path: worktree_path.as_deref(),
            resolves_the_base_merge: *resolves_the_base_merge,
        },
        UpstreamSyncFailure::Blocked { raw_error, .. } => SyncStepNext::Fail(raw_error),
    }
}

/// The `sync` node's words for a base merge that did not land after a
/// reconcile conflict was resolved.
///
/// The reconcile is on the branch and the base is not — the one state this node
/// can be left in that neither a completed sync nor an ordinary conflict
/// describes. A second resolver turn is deliberately not spawned for it: the
/// first was budgeted as this node's one resolution, and a node that keeps
/// resolving until git stops complaining is a retry loop with an agent inside
/// it.
pub fn base_merge_refusal(base_branch: &str, failure: &UpstreamSyncFailure) -> String {
    match step_next(failure) {
        SyncStepNext::Fail(raw_error) => raw_error.to_string(),
        SyncStepNext::Resolve { files, .. } => format!(
            "'{base_branch}' was not merged. The divergence with origin was reconciled and              that resolution landed, but merging origin/{base_branch} into the result then              left {count} file(s) conflicted: {paths}. Resolve that merge and sync again.",
            count = files.len(),
            paths = files
                .iter()
                .map(|f| f.path.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }
}

/// The view a completed sync attempt renders as.
pub fn view_for(result: Result<UpstreamSyncOutcome, UpstreamSyncFailure>) -> SyncOutcomeView {
    match result {
        Ok(outcome) => SyncOutcomeView::Ok {
            merge_commit_sha: outcome.merge_commit_sha,
            changed: outcome.changed,
        },
        Err(UpstreamSyncFailure::Conflict { report, .. }) => SyncOutcomeView::Conflict {
            conflict_files: report.files,
            raw_error: report.raw_error,
        },
        Err(UpstreamSyncFailure::Blocked { stage, raw_error }) => {
            SyncOutcomeView::Blocked { stage, raw_error }
        }
    }
}

/// The same attempt, as the "Sync with main" press is answered.
///
/// [`SyncBlockedStage::TurnInFlight`] is the one outcome that must not come
/// back as a view. The pane throws a `SyncOutcomeView` away and re-reads the
/// session row, and the row a refused press would re-read belongs to the turn
/// that refused it — so it reads `syncing`, and a press that did nothing at all
/// renders as one that started a merge. An `Err` reaches the error bus, which
/// is where a refusal the user can act on belongs.
pub fn command_view(
    result: Result<UpstreamSyncOutcome, UpstreamSyncFailure>,
) -> Result<SyncOutcomeView, String> {
    match result {
        Err(UpstreamSyncFailure::Blocked {
            stage: SyncBlockedStage::TurnInFlight,
            raw_error,
        }) => Err(raw_error),
        other => Ok(view_for(other)),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/sync_failure.rs"]
mod tests;
