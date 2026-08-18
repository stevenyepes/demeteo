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
//! `feature_syncs` (V9) stays the append-only audit of attempts and is not
//! read here; this table holds the single mutable row a feature is allowed.

use serde::{Deserialize, Serialize};

use crate::domain::ids::FeatureId;
use crate::domain::models::ConflictFile;
use crate::domain::sync_session::SyncSessionStatus;

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
    pub bump_attempts: bool,
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
