//! What a feature's live sync is *actually* in, given what the working tree
//! says. See [`crate::domain`].
//!
//! The stored status is a claim made by whichever process last wrote it, and
//! the failure this module exists for is that the process which should write
//! the closing status is exactly the one that dies: a killed resolver leaves
//! `resolving` forever, and a user who finishes the merge in their own editor
//! tells the table nothing. The worktree is the authority and the row is the
//! index, so a read that trusts the row alone reproduces the bug it replaced —
//! answering "is there a conflict?" from a record instead of from git.

use serde::{Deserialize, Serialize};

/// The state a feature's sync is in, as the schema spells it (V43).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncSessionStatus {
    /// The merge is running right now.
    Syncing,
    /// `origin/<base>` had nothing the feature branch did not already have.
    UpToDate,
    /// The merge landed cleanly.
    Merged,
    /// The sync stopped before it reached a merge verdict
    /// ([`crate::domain::sync_failure`]). Nothing is conflicted.
    Blocked,
    /// The merge ran and left unmerged paths.
    Conflicted,
    /// An agent is working through the conflicted tree.
    Resolving,
    /// The conflicted tree was resolved and committed.
    Resolved,
    /// The resolution turn ran and did not produce a resolved tree.
    ResolutionFailed,
    /// The user gave up on this sync; the merge was aborted and the worktree
    /// discarded.
    Aborted,
}

impl SyncSessionStatus {
    /// The stable lowercase identifier used on the wire and in the DB.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Syncing => "syncing",
            Self::UpToDate => "up_to_date",
            Self::Merged => "merged",
            Self::Blocked => "blocked",
            Self::Conflicted => "conflicted",
            Self::Resolving => "resolving",
            Self::Resolved => "resolved",
            Self::ResolutionFailed => "resolution_failed",
            Self::Aborted => "aborted",
        }
    }

    /// Parse a stored status. `None` for anything unknown, so a row written by
    /// a newer build degrades rather than panicking — mirrors
    /// [`EffortLevel::parse`](crate::domain::models::EffortLevel::parse).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "syncing" => Some(Self::Syncing),
            "up_to_date" => Some(Self::UpToDate),
            "merged" => Some(Self::Merged),
            "blocked" => Some(Self::Blocked),
            "conflicted" => Some(Self::Conflicted),
            "resolving" => Some(Self::Resolving),
            "resolved" => Some(Self::Resolved),
            "resolution_failed" => Some(Self::ResolutionFailed),
            "aborted" => Some(Self::Aborted),
            _ => None,
        }
    }

    /// Nothing is waiting on this session and nothing on disk belongs to it,
    /// so no observation can contradict it.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::UpToDate | Self::Merged | Self::Aborted)
    }
}

/// What the working tree says, independent of what the row claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncWorkspaceProbe {
    /// The directory named by `worktree_path` is still there.
    pub worktree_exists: bool,
    /// `MERGE_HEAD` resolves — a merge is open and unfinished.
    pub merge_in_progress: bool,
    /// `git status --porcelain` had something to say.
    pub dirty: bool,
}

/// The stored status corrected by what is on disk.
///
/// `probe` is `None` when the session names no worktree to look at, which is
/// not the same as looking and finding nothing: a sync that has not
/// provisioned one yet would otherwise read as abandoned on its first poll.
pub fn reconcile(
    stored: SyncSessionStatus,
    probe: Option<&SyncWorkspaceProbe>,
) -> SyncSessionStatus {
    use SyncSessionStatus::*;

    if stored.is_terminal() {
        return stored;
    }
    let Some(probe) = probe else {
        return stored;
    };
    if !probe.worktree_exists {
        // The tree the session was about is gone — force-removed by a later
        // sync, cleaned up by hand, or never re-created after a restart.
        // Nothing remains to resolve, continue or abort.
        return Aborted;
    }
    match stored {
        // A `resolving` row is only ever *read* by a process that is not the
        // one which wrote it, so seeing one means its writer is gone. An open
        // merge is then a conflict waiting for someone, not work in progress.
        Resolving if probe.merge_in_progress => Conflicted,
        Resolved if probe.merge_in_progress => Conflicted,
        // The merge is closed and the tree is clean, so somebody committed the
        // resolution — an agent that staged and committed on its own, or the
        // user in their own editor. `Blocked` is deliberately not in this arm:
        // a push that failed leaves exactly this shape, and nothing about it
        // was ever conflicted.
        Conflicted | Resolving | ResolutionFailed if !probe.merge_in_progress && !probe.dirty => {
            Resolved
        }
        other => other,
    }
}

#[cfg(test)]
#[path = "../../tests/domain/sync_session.rs"]
mod tests;
