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
    /// origin failed. The only stage where work exists and is at risk.
    Push,
    /// The feature's project repository row could not be resolved, so no git
    /// command was ever issued.
    RepoContext,
    /// The previous sync's resolution is still committed on the branch and
    /// unpublished — or the row that would rule that out could not be read — so
    /// no new sync was started
    /// ([`resync_refusal`](crate::domain::sync_session::resync_refusal)).
    HeldResolution,
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
            Self::RepoContext => "repo_context",
            Self::HeldResolution => "held_resolution",
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
            "repo_context" => Some(Self::RepoContext),
            "held_resolution" => Some(Self::HeldResolution),
            _ => None,
        }
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
    },
    Fail(&'a str),
}

pub fn step_next(failure: &UpstreamSyncFailure) -> SyncStepNext<'_> {
    match failure {
        UpstreamSyncFailure::Conflict {
            report,
            worktree_path,
        } => SyncStepNext::Resolve {
            files: &report.files,
            worktree_path: worktree_path.as_deref(),
        },
        UpstreamSyncFailure::Blocked { raw_error, .. } => SyncStepNext::Fail(raw_error),
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

#[cfg(test)]
#[path = "../../tests/domain/sync_failure.rs"]
mod tests;
