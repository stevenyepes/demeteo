//! Worktree operations port.
//!
//! Provides abstract access to Git worktree operations such as cloning,
//! provisioning worktrees, checking repository state, and syncing with upstream.

use crate::domain::branch_listing::BranchOption;
use crate::domain::feature_origin::Refspec;
use crate::domain::models::{WorktreeInfo, WorktreeStrategy};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Repository and project roots resolved from Demeteo-owned records.
///
/// This is deliberately opaque outside `demeteo-core`: a terminal caller must
/// prove project and repository ownership through application policy before it
/// can request filesystem mutation. The strings retain the target host's path
/// syntax; a desktop process must not reinterpret a remote path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedWorktreeTarget {
    machine_id: Option<String>,
    repository_dir: String,
    project_root: String,
}

impl TrustedWorktreeTarget {
    /// Construct a target after project/repository ownership and host selection
    /// have been resolved by application policy.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the port is a designed contract with no production caller yet; a \
                      cfg(test) caller fulfils it under --all-targets and would make the \
                      expectation itself the lint"
        )
    )]
    pub(crate) fn from_resolved(
        machine_id: Option<String>,
        repository_dir: String,
        project_root: String,
    ) -> Self {
        Self {
            machine_id,
            repository_dir,
            project_root,
        }
    }

    pub fn machine_id(&self) -> Option<&str> {
        self.machine_id.as_deref()
    }

    pub fn repository_dir(&self) -> &str {
        &self.repository_dir
    }

    pub fn project_root(&self) -> &str {
        &self.project_root
    }
}

/// A terminal-worktree creation scoped to a [`TrustedWorktreeTarget`].
#[derive(Debug, Clone)]
pub struct CreateTrustedTerminalWorktreeRequest {
    pub target: TrustedWorktreeTarget,
    pub terminal: TerminalWorktreeRequest,
}

/// The terminal worktree created by [`TrustedWorktreePort`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedTerminalWorktreeCreated {
    pub worktree: WorktreeInfo,
    pub base_ref: String,
}

/// A terminal-worktree removal scoped to a [`TrustedWorktreeTarget`].
///
/// `worktree_name` is a relative name below the target's terminal area, never
/// a caller-selected absolute path.
#[derive(Debug, Clone)]
pub struct RemoveTrustedTerminalWorktreeRequest {
    pub target: TrustedWorktreeTarget,
    pub worktree_name: String,
    pub force: bool,
}

/// Evidence of the terminal worktree that was retired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedTerminalWorktreeRemoved {
    pub worktree: WorktreeInfo,
}

/// Materialize the known dependency-cache directories for one trusted
/// worktree from its feature-scoped cache root.
///
/// Both paths originate in Demeteo's worktree derivation, rather than in an
/// agent prompt or terminal UI. The directory set itself is fixed by the port
/// contract; callers cannot widen it with arbitrary path names.
#[derive(Debug, Clone)]
pub struct MaterializeDependencyCacheRequest {
    pub target: TrustedWorktreeTarget,
    pub worktree_dir: String,
    pub feature_cache_dir: String,
}

/// The dependency-cache directories materialized for a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyCacheMaterialization {
    /// Well-known dependency directories made available in the worktree.
    pub materialized: Vec<String>,
    /// Well-known dependency directories absent from the feature cache.
    pub absent: Vec<String>,
}

/// Filesystem mutations beneath a Demeteo-owned worktree root.
///
/// This is intentionally separate from [`WorktreeOpsPort`]. It is the narrow
/// future contract for operations whose safety depends on trusted-root and
/// no-follow handling. The two adapters that implement it are not reached from
/// any production path: the local one is Unix-only and launches Git directly
/// rather than through [`ExecutionPort`](crate::ports::execution::ExecutionPort),
/// so wiring it would make a step behave differently by transport. Its complete
/// security and transport requirements are in `docs/TRUSTED_WORKTREE.md`.
#[async_trait]
pub trait TrustedWorktreePort: Send + Sync {
    /// Create a terminal worktree below the target's derived terminal area.
    ///
    /// Every existing component entered below the trusted root must be checked
    /// without following symlinks or platform reparse points. The result must
    /// report the physical worktree path and the base ref Git actually used.
    async fn create_terminal_worktree(
        &self,
        request: CreateTrustedTerminalWorktreeRequest,
    ) -> Result<TrustedTerminalWorktreeCreated, String>;

    /// Remove exactly one terminal worktree below the target's derived area.
    ///
    /// Implementations must re-derive the destination from `worktree_name` and
    /// validate it against Git's current worktree registration before removal.
    async fn remove_terminal_worktree(
        &self,
        request: RemoveTrustedTerminalWorktreeRequest,
    ) -> Result<TrustedTerminalWorktreeRemoved, String>;

    /// Materialize only the contract's known dependency-cache directories.
    ///
    /// Materialized build outputs are feature-scoped; shared download caches
    /// belong behind a separate capability and are not exposed by this method.
    async fn materialize_dependency_cache(
        &self,
        request: MaterializeDependencyCacheRequest,
    ) -> Result<DependencyCacheMaterialization, String>;
}

/// The caller-controlled half of a terminal-worktree creation.
///
/// Bundled rather than passed positionally: all three travel together from the
/// Tauri command through application policy into the adapter, and every one of
/// them is a `String` the user typed or picked. Two adjacent untrusted strings
/// of the same type is a swap the compiler cannot see.
#[derive(Debug, Clone)]
pub struct TerminalWorktreeRequest {
    /// The new branch to create. Must not already exist.
    pub branch: String,
    /// The branch to cut from. `None` leaves the start point at the primary
    /// checkout's HEAD, which is whatever the user last left it on.
    pub base_branch: Option<String>,
    /// The directory name below the repository's terminal area.
    pub worktree_name: String,
}

/// A terminal worktree that now exists, and where its branch came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalWorktreeCreated {
    pub worktree: WorktreeInfo,
    /// The start point Git was actually given: `origin/<base>` once the fetch
    /// reached origin, the local `<base>` when it did not, `HEAD` when the
    /// request named no base.
    ///
    /// Reported rather than inferred, because it is the whole answer to "am I
    /// starting from something stale" — and the fallback to a local ref happens
    /// exactly when the network was the thing that failed, which is when a
    /// caller assuming `origin/<base>` would be most wrong.
    pub base_ref: String,
}

/// How far a feature branch has drifted from the base it will merge into.
///
/// Both counts are `Option` because the three ways a `rev-list` can answer
/// nothing — zero commits, an unresolvable ref, a dead transport — are three
/// different facts and only the first one means "up to date". Collapsing them
/// is how a branch nobody could measure renders as current, which is the one
/// answer a staleness signal must never invent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchDivergence {
    /// Commits `origin/<base>` has that the feature branch does not.
    pub behind: Option<u64>,
    /// Commits the feature branch has that `origin/<base>` does not.
    pub ahead: Option<u64>,
}

impl BranchDivergence {
    /// Neither side was measured. Spelled out so a construction site reads as
    /// the assertion it is rather than as two fields somebody forgot to fill.
    pub const fn unknown() -> Self {
        Self {
            behind: None,
            ahead: None,
        }
    }
}

/// Result of a successful feature branch sync.
#[derive(Debug, Clone)]
pub struct SyncOutcome {
    /// The tip the sync left the branch on — the merge commit, or the tip it
    /// found when there was nothing to merge. `None` when `rev-parse` did not
    /// answer, which is not a commit named `""` and may not be stored as one.
    pub merge_commit_sha: Option<String>,
    /// `false` when `origin/<default>` didn't exist or had no new
    /// commits since the last sync.
    pub changed: bool,
    /// The feature branch's tip before the merge. Reported rather than
    /// re-derived, because `merge_commit^` stops being it the moment anything
    /// commits on top — and by then nothing can recover it. `None` when the
    /// read failed: a base that was never measured is not the same as one that
    /// resolved to nothing, and only the first may be stored as unknown.
    pub head_before: Option<String>,
}

/// Why a feature-branch sync did not land. Which variant it is cannot be
/// inferred from the payload — see [`crate::domain::sync_failure`].
#[derive(Debug, Clone)]
pub enum SyncFailure {
    /// The merge ran and left unmerged paths. `worktree_path` is where the
    /// conflicted index lives, and the resolution agent must run there;
    /// `None` when the probe for it failed.
    Conflict {
        files: Vec<crate::domain::models::ConflictFile>,
        raw_error: String,
        worktree_path: Option<String>,
        /// The feature branch's tip before the merge, on the same terms as
        /// [`SyncOutcome::head_before`] — a resolution commits on top of the
        /// merge, so this is the only base a review diff can use. `None` when
        /// the read for it failed, which is not the same as a branch with no
        /// tip and may not be flattened into one.
        head_before: Option<String>,
    },
    /// No merge was attempted, or one was and never reached a verdict, or its
    /// result could not be published. Nothing is known to be conflicted, so
    /// there is nothing for an agent to do.
    Blocked {
        stage: crate::domain::sync_failure::SyncBlockedStage,
        raw_error: String,
        /// A worktree this attempt provisioned and did not clean up, when there
        /// is one. `Push` and `Merge` both leave one: the cleanup in
        /// `sync_feature_with_upstream` runs only on success, and the push
        /// failure returns before reaching it. Carrying it is what lets the
        /// session name the tree, and `sync_abort` reclaim it — otherwise the
        /// only thing that ever removes it is the next sync's force-remove.
        worktree_path: Option<String>,
        /// As [`SyncFailure::Conflict::head_before`]. Known at every stage after
        /// the refs are read, and a `Push` failure leaves a real merge commit
        /// sitting on top of it.
        head_before: Option<String>,
        /// The merge this attempt committed before it was blocked — `Push` and
        /// nothing else, because it is the only stage reached after the merge
        /// succeeded. Without it the session names a merge it cannot identify,
        /// and publishing one has no sha to confirm against origin afterwards,
        /// which is the only evidence a push may be recorded on.
        merge_commit_sha: Option<String>,
    },
}

/// Told where a sync's merge is about to run, the moment there is an answer.
///
/// The session row is opened before the fetch, because a sync cut short is the
/// case it exists for — and until this existed it named no tree, so the
/// interrupted sync was the one state nothing could probe, `reconcile` passed
/// through untouched and every intervention refused. The path is known here
/// and nowhere else until the whole call returns, which for an interrupted sync
/// is never.
pub trait SyncWorktreeObserver: Send + Sync {
    fn provisioned(&self, worktree_path: &str);
}

/// For a caller with no row to keep — the port's own trait method, and the
/// tests that drive git directly.
impl SyncWorktreeObserver for () {
    fn provisioned(&self, _worktree_path: &str) {}
}

/// Result of collapsing a feature branch's commits into one.
#[derive(Debug, Clone, PartialEq)]
pub enum SquashOutcome {
    /// The branch now points at a single new commit.
    Squashed {
        sha: String,
        /// How many commits were collapsed.
        collapsed: u32,
        /// Ref holding the pre-squash tip, so the rewrite is undoable.
        backup_ref: String,
    },
    /// The branch adds nothing to the default branch (no net change), so
    /// there is nothing to squash and nothing worth opening a PR for.
    NothingToSquash,
}

/// The repo's `commit-msg` hook rejected a proposed message.
#[derive(Debug, Clone)]
pub struct CommitMessageRejected {
    /// The hook's own output (e.g. commitlint's rule list) — fed back to
    /// the agent so it can repair the message.
    pub hook_output: String,
}

#[async_trait]
pub trait WorktreeOpsPort: Send + Sync {
    /// Check if the repository is dirty.
    async fn check_repo_dirty(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<(bool, bool), String>;

    /// Retrieve the HEAD branch name.
    async fn get_head_branch(&self, machine_id: Option<&str>, repo_dir: &str) -> Option<String>;

    /// List all git worktrees for the repository.
    async fn list_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<WorktreeInfo>, String>;

    /// Create a user-requested linked worktree without altering or reclaiming
    /// any existing worktree state.
    ///
    /// This creates a **new** branch. Supplying a branch that already exists is
    /// an error; implementations must not reuse, reset, or check out that
    /// branch in a new worktree.
    ///
    /// When the request names a base, implementations fetch it from origin
    /// first and cut from `origin/<base>`, falling back to the local `<base>`
    /// only when origin has no such ref. The primary checkout's HEAD is never
    /// silently used as the start point in that case: a session started from a
    /// stale base is the failure this is here to prevent, so the ref actually
    /// used comes back in [`TerminalWorktreeCreated::base_ref`].
    ///
    /// `project_root` is the Demeteo-owned root of the project and must
    /// already exist on the target host; implementations derive the worktree
    /// destination *below* it. Every field of `request` stays untrusted — the
    /// caller never gets to choose an absolute destination.
    async fn create_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        request: &TerminalWorktreeRequest,
    ) -> Result<TerminalWorktreeCreated, String>;

    /// Retire one terminal worktree the user is done with.
    ///
    /// `worktree_path` is untrusted — it arrives from a UI holding a listing
    /// that may be seconds out of date, and this method is one `git worktree
    /// remove --force` away from deleting a pipeline's checkout or the primary
    /// one. Implementations must therefore re-derive the terminal area from
    /// `project_root` and refuse any path
    /// [`list_terminal_worktrees`](Self::list_terminal_worktrees) would not
    /// have offered, rather than trusting the caller's path.
    ///
    /// The branch is deliberately left behind: the worktree is a directory the
    /// user can recreate, and the commits in it are not.
    ///
    /// `force` maps to git's own `--force`, which is what a worktree holding
    /// modified or untracked files needs. Without it, git's refusal comes back
    /// as the error — a caller is expected to surface that and let the user
    /// decide, never to retry with `force` on its own.
    async fn remove_terminal_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
        worktree_path: &str,
        force: bool,
    ) -> Result<(), String>;

    /// The branches a new terminal worktree may be based on.
    ///
    /// Read from refs already on the target host — no fetch. Opening a picker
    /// must not block on the network, and the fetch that makes a base current
    /// happens inside
    /// [`create_terminal_worktree`](Self::create_terminal_worktree), where the
    /// user has already committed to a base.
    async fn list_terminal_branches(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<Vec<BranchOption>, String>;

    /// The linked worktrees of `repo_dir` that belong to the terminal area
    /// below `project_root`, and nothing else.
    ///
    /// Separate from [`list_worktrees`](Self::list_worktrees) rather than a
    /// filter over it, because the classification needs the primary checkout's
    /// path and that method deliberately withholds it — three of its callers
    /// `worktree remove --force` whatever survives their own filter, so the
    /// main checkout must not be in what they filter.
    ///
    /// An error, not an empty list, when the area cannot be derived: an empty
    /// list is what a healthy repository with no terminal worktrees returns.
    async fn list_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        project_root: &str,
    ) -> Result<Vec<WorktreeInfo>, String>;

    /// Retire terminal worktrees left at the location this feature used before
    /// the area moved out of `repos/`, returning how many were unregistered.
    ///
    /// Unregistering is the point: those directories sit where
    /// `application::bootstrap` reclaims the tree, so a plain delete leaves
    /// `.git/worktrees` entries behind and a later add of the same name fails
    /// against a worktree Git still believes in. Implementations must leave the
    /// current-location worktrees alone.
    async fn cleanup_legacy_terminal_worktrees(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<usize, String>;

    /// Detect the worktree strategy and return it.
    async fn detect_worktree_strategy(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
    ) -> Result<WorktreeStrategy, String>;

    /// Run clone operation.
    async fn clone_repository(
        &self,
        machine_id: Option<&str>,
        provider_id: &str,
        repo_path: &str,
        target_dir: &str,
    ) -> Result<(), String>;

    /// Create a feature branch.
    async fn create_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        default_branch: &str,
        branch_name: &str,
    ) -> Result<(), String>;

    /// Bring exactly one refspec down from origin, reporting whether it
    /// arrived.
    ///
    /// Whether that report stops the run is
    /// [`BranchCut`](crate::domain::feature_origin::BranchCut)'s decision and
    /// not this method's, which is why it reports rather than tolerates.
    ///
    /// The refspec is passed after `--`; see [`Refspec`] for what that is
    /// holding shut.
    async fn fetch_origin_refspec(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        refspec: &Refspec,
    ) -> Result<(), String>;

    /// Point `branch_name` at `start_point`, which must already resolve.
    ///
    /// The counterpart to
    /// [`create_feature_branch`](Self::create_feature_branch) for a run whose
    /// origin named its own start point. There is no fallback ref to try:
    /// falling back is how a run started somewhere the user chose silently
    /// becomes a run started from the default branch.
    async fn cut_branch_at(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        start_point: &str,
        branch_name: &str,
    ) -> Result<(), String>;

    /// Provision a subtask worktree.
    async fn provision_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<String, String>;

    /// Clean up a subtask worktree.
    async fn cleanup_subtask_worktree(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<(), String>;

    /// Delete a branch (and optionally any subtask branches and prune worktrees).
    async fn branch_delete(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        branch: &str,
    ) -> Result<(), String>;

    /// Merge a subtask branch.
    async fn merge_subtask(
        &self,
        machine_id: Option<&str>,
        worktree_dir: &str,
        branch: &str,
        subtask_id: &str,
    ) -> Result<(), String>;

    /// Sync feature branch with upstream default branch.
    ///
    /// Reports no worktree: a caller holding a session row wants
    /// [`SyncWorktreeObserver`] and reaches `GitOpsHelper` directly for it.
    async fn sync_feature_with_upstream(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        base_branch: &str,
    ) -> Result<SyncOutcome, SyncFailure>;

    /// Run the repo's own `commit-msg` hook against a proposed message,
    /// without committing anything.
    ///
    /// This is how the finalize step lets a target repo's commitlint judge
    /// the squashed commit message *before* the message is used, so a
    /// rejection becomes feedback for the authoring agent instead of a
    /// failed commit. `Ok(())` when the repo installs no `commit-msg` hook.
    async fn validate_commit_message(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        message: &str,
    ) -> Result<(), CommitMessageRejected>;

    /// Collapse every commit the feature branch adds on top of `base_ref`
    /// into a single commit carrying `message`.
    ///
    /// `base_ref` is where the run started, which is the project's default
    /// branch only for a run that started there — see
    /// [`FeatureOrigin::squash_base`](crate::domain::feature_origin::FeatureOrigin::squash_base).
    async fn squash_feature_branch(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
        base_ref: &str,
        message: &str,
    ) -> Result<SquashOutcome, String>;

    /// Restore a branch to its pre-squash tip from the backup ref written
    /// by [`squash_feature_branch`](Self::squash_feature_branch).
    async fn restore_pre_squash(
        &self,
        machine_id: Option<&str>,
        repo_dir: &str,
        feature_branch: &str,
    ) -> Result<(), String>;
}
