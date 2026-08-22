//! One feature's live sync (V43), which is the thing the UI, the abort command
//! and the resolver all need to agree on.
//!
//! **The row is a claim; the working tree is the authority.** Nothing here
//! reconciles — the port stores and returns what was written, and every reader
//! must correct it against a probe of the worktree
//! ([`crate::domain::sync_session::reconcile`]) before believing a
//! non-terminal status. `application::sync_session::get_reconciled` is the one
//! place that does both, and is what a command should call.
//!
//! The tree is not the only authority, and reading it as if it were is what
//! aimed a worktree delete at a live agent: a merge in progress looks the same
//! whether the process holding it is running or died an hour ago. Whether
//! anything still is, is the second observation
//! ([`crate::domain::sync_session::sync_liveness`]), and the same one call
//! supplies it.
//!
//! `feature_syncs` (V9) stays the append-only audit of attempts and is not
//! read here; this table holds the single mutable row a feature is allowed.

use serde::{Deserialize, Serialize};

use crate::domain::ids::FeatureId;
use crate::domain::models::ConflictFile;
use crate::domain::sync_failure::SyncBlockedStage;
use crate::domain::sync_session::{SyncResolution, SyncSessionStatus};

/// One feature's live sync, as the row holds it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSession {
    pub feature_id: String,
    pub machine_id: String,
    /// The clone the sync worktree was cut from, and the `-C` target every
    /// teardown needs. May equal `worktree_path`.
    pub repo_dir: String,
    pub feature_branch: String,
    pub base_branch: String,
    pub status: SyncSessionStatus,
    pub worktree_path: Option<String>,
    /// The feature branch's tip before the merge — the base a review diff is
    /// computed from, recorded while it is still reachable.
    pub head_before: Option<String>,
    pub merge_commit_sha: Option<String>,
    /// Empty and "never measured" are both spelled `[]` here; the column
    /// distinguishes them and nothing downstream has needed to.
    pub conflict_files: Vec<ConflictFile>,
    pub raw_error: Option<String>,
    /// Where a [`SyncSessionStatus::Blocked`] session stopped (migration V46),
    /// or `None` on any other status and on a row this build's vocabulary
    /// cannot name. Not derivable from anything else on the row: `raw_error`
    /// is git's prose and `merge_commit_sha` is set for a stage that failed
    /// after committing as readily as for one that never merged.
    pub blocked_stage: Option<SyncBlockedStage>,
    /// When the resolution reached origin, or `None` while it is only on the
    /// branch. Not a [`SyncSessionStatus`] because no probe of the working tree
    /// can answer it — see migration V45.
    pub pushed_at: Option<i64>,
    pub attempts: u32,
    pub created_at: i64,
    pub updated_at: i64,
}

/// The fields one transition may change.
///
/// `Option<Option<T>>` on the same terms as [`FeaturePatch`](crate::ports::db::FeaturePatch):
/// `None` leaves the column alone, `Some(None)` clears it, `Some(Some(v))`
/// sets it.
#[derive(Debug, Default, Clone)]
pub struct SyncSessionPatch {
    pub status: Option<SyncSessionStatus>,
    pub worktree_path: Option<Option<String>>,
    pub head_before: Option<Option<String>>,
    pub merge_commit_sha: Option<Option<String>>,
    pub conflict_files: Option<Vec<ConflictFile>>,
    pub raw_error: Option<Option<String>>,
    pub blocked_stage: Option<Option<SyncBlockedStage>>,
    pub pushed_at: Option<Option<i64>>,
    pub bump_attempts: bool,
}

impl SyncSessionPatch {
    /// The transition a resolution turn's outcome makes to the row.
    ///
    /// Its own function because it decides *what the row should say* — three of
    /// its four columns are written as clears, and a clear is the half no
    /// happy-path assertion reaches — while the only caller is an `async fn`
    /// mid-way through a sync (AGENTS.md §3).
    pub fn from_resolution(resolution: &SyncResolution, now: i64) -> Self {
        Self {
            status: Some(resolution.status()),
            merge_commit_sha: match resolution {
                SyncResolution::Succeeded {
                    merge_commit_sha, ..
                } => Some(Some(merge_commit_sha.clone())),
                _ => None,
            },
            // A resolved sync that discarded its worktree leaves a row still
            // naming it reading back as an abandoned sync: the probe finds the
            // directory gone, which is the one observation `reconcile` treats
            // as terminal. Clearing it is also what stops a later abort aiming
            // a delete at a path something else may have re-provisioned since.
            // Only on the caller's *observation* that the tree went, though —
            // see [`SyncResolution::Succeeded`].
            worktree_path: match resolution {
                SyncResolution::Succeeded {
                    worktree_discarded: true,
                    ..
                } => Some(None),
                _ => None,
            },
            raw_error: match resolution {
                SyncResolution::Failed { reason } => Some(Some(reason.clone())),
                _ => None,
            },
            // Written in both directions from the outcome alone, because the
            // turn is the only thing that can know: the commit is on the branch
            // either way, and nothing about the tree afterwards distinguishes a
            // resolution origin has from one it has not. A `Succeeded` that did
            // not publish therefore says so rather than leaving whatever the
            // column held to answer in its place.
            pushed_at: match resolution {
                SyncResolution::Succeeded {
                    published: true, ..
                } => Some(Some(now)),
                SyncResolution::Succeeded {
                    published: false, ..
                } => Some(None),
                _ => None,
            },
            ..Self::default()
        }
    }
}

/// A session as the UI reads it: the row, plus the one decision the UI is not
/// allowed to make for itself.
///
/// `user_may_intervene` is computed here rather than in the frontend because it
/// is a policy question — who owns this worktree right now — and AGENTS.md §3
/// keeps those in `domain/`. Spelled as a condition in TSX it would drift from
/// the workflow step that writes the states it reads.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncSessionView {
    #[serde(flatten)]
    pub session: SyncSession,
    pub user_may_intervene: bool,
}

pub trait SyncSessionPort: Send + Sync {
    /// Open the feature's session, replacing whatever the previous one
    /// claimed. Idempotent on `feature_id`: a feature has at most one sync in
    /// flight and the primary key is what enforces it, so an abandoned session
    /// is overwritten by the next attempt rather than accumulating.
    fn open(&self, session: &SyncSession) -> Result<(), String>;
    /// The row as stored. Callers **must** reconcile a non-terminal status
    /// against the working tree before believing it; see the module docs.
    fn get(&self, feature_id: &FeatureId) -> Result<Option<SyncSession>, String>;
    fn update(
        &self,
        feature_id: &FeatureId,
        patch: &SyncSessionPatch,
        now: i64,
    ) -> Result<(), String>;
    /// Forget the session entirely.
    ///
    /// Not how a sync ends: abandoning one records
    /// [`SyncSessionStatus::Aborted`] and keeps the row, because the states the
    /// user can still be shown — and the audit of how a feature's syncs have
    /// been going — both live in it. This exists for a caller that wants the
    /// feature to have no sync history at all, and the FK cascade already covers
    /// the only one there is: deleting the feature.
    fn close(&self, feature_id: &FeatureId) -> Result<(), String>;
}

#[cfg(test)]
#[path = "../../tests/ports/sync_session.rs"]
mod tests;
