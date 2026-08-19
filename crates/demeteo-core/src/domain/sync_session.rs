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

use crate::domain::sync_failure::SyncBlockedStage;

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

/// What a resolution turn reports back to the session it is working on.
///
/// Narrower than [`SyncSessionStatus`] on purpose: a resolver is only ever in
/// three of those states, and a caller handed the whole vocabulary can file a
/// verdict no resolution produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncResolution {
    /// A turn is about to start. Writing this is what stops the session from
    /// claiming `conflicted` while an agent holds the worktree.
    Started,
    /// The conflicts are gone and committed.
    Succeeded {
        merge_commit_sha: String,
        /// Whether that commit reached origin, which is the whole of the
        /// difference between a finished sync and one waiting for a look
        /// ([`publish_policy`]). Carried on the outcome rather than derived
        /// from it because the turn is the only thing that knows: the commit
        /// is on the branch either way, and nothing about the working tree
        /// afterwards distinguishes the two.
        published: bool,
        /// Whether the throwaway worktree was *observed* to be gone after the
        /// teardown, which is what decides whether the row may stop naming it.
        ///
        /// The teardown is best-effort and reports nothing, so a row blanked on
        /// its say-so can leave a directory on disk that no row names and no
        /// reader revisits — the leak
        /// [`abort`](crate::application::sync_session::abort) refuses to create
        /// by returning an error instead. Two paths clearing one column on
        /// opposite rules is what the flag removes.
        worktree_discarded: bool,
    },
    /// The turn ran and did not produce a resolved tree.
    Failed { reason: String },
}

impl SyncResolution {
    /// The status this outcome puts the session in.
    pub fn status(&self) -> SyncSessionStatus {
        match self {
            Self::Started => SyncSessionStatus::Resolving,
            Self::Succeeded { .. } => SyncSessionStatus::Resolved,
            Self::Failed { .. } => SyncSessionStatus::ResolutionFailed,
        }
    }
}

/// Whether the *user* may act on this session, or whether it belongs to
/// something already working on it.
///
/// Persisting the session is what made this question exist. Before, a conflict
/// banner only appeared if the user had personally clicked Sync in that
/// session, so the only sync they could see was one nobody else was driving.
/// A session read back from the table has no such guarantee: the workflow's own
/// `sync` step conflicts and resolves without the user involved, and the
/// destructive affordances aimed at it — abort deletes the worktree an agent is
/// mid-write in, resolve puts a second agent in the same tree — are both worse
/// than doing nothing.
///
/// The session's own status is the one input that cannot answer this. Between
/// the merge failing and the turn that takes it over recording itself, the row
/// legitimately reads `conflicted` while something else holds the worktree —
/// which is why [`SyncStanding`] carries both traces of a live turn instead
/// ([`sync_liveness`]).
pub fn user_may_intervene(standing: SyncStanding<'_>) -> bool {
    [
        SyncIntervention::Abort,
        SyncIntervention::Resolve,
        SyncIntervention::Publish,
        SyncIntervention::Discard,
    ]
    .into_iter()
    .any(|action| intervention_refusal(action, standing).is_none())
}

/// Everything an intervention is judged against: what the session claims, what
/// has been published of it, where it stopped, and whether anything else is
/// still driving the branch.
///
/// They travel together and are meaningless apart — a status without the
/// feature's own status cannot tell a conflict the user owns from one a run is
/// mid-way through resolving, neither of them can tell a resolution that still
/// needs pushing from one already on origin, and none of them can tell the one
/// blocked sync holding a real merge from the six that hold nothing.
#[derive(Debug, Clone, Copy)]
pub struct SyncStanding<'a> {
    pub status: SyncSessionStatus,
    /// The resolution reached origin. A row field rather than a status: no
    /// probe of the working tree can observe it (migration V45).
    pub published: bool,
    /// Where a [`SyncSessionStatus::Blocked`] session stopped, or `None` on any
    /// other status and on a row written before migration V46 recorded it.
    /// Meaningless for every other status and never consulted there.
    pub blocked_stage: Option<SyncBlockedStage>,
    /// The feature's status, which is what says whether a driver still owns
    /// the branch.
    pub feature_status: &'a str,
    /// Whether anything is running this sync *now*
    /// ([`sync_liveness`]), which no status can stand in for.
    ///
    /// A turn writes `resolving` after it has already claimed the worktree —
    /// `feature_resolve_sync_conflicts` runs a preflight of several round trips
    /// in between, and the write itself is best-effort — so between the two the
    /// row still reads `conflicted` while an agent edits in that directory.
    /// Judging the refusals on status alone offered Abort for that window:
    /// `git merge --abort`, `worktree remove --force`, `remove_dir_all`, aimed
    /// at a live tree.
    pub liveness: SyncLiveness,
}

/// What the user is asking to do to a sync that is not running.
///
/// The two affordances do not accept the same sessions, and collapsing them
/// into one predicate is what let the resolve IPC reach a `Blocked` row: a
/// sync that stopped before it reached a merge has no conflicts, so resolving
/// it can only fail — and failing it rewrites the stored `UpstreamSyncFailure`
/// text, which is the one thing a blocked row exists to keep. Aborting the same
/// row is legitimate; it holds a real unpublished merge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncIntervention {
    /// Undo the merge, discard the worktree, close the session.
    Abort,
    /// Put an agent in the conflicted worktree.
    Resolve,
    /// Send a resolution that is only on the branch to origin.
    Publish,
    /// Put the branch back where the merge found it and give up on the sync.
    Discard,
}

/// Why this sync is not the user's to act on, or `None` when it is.
///
/// The same decision as [`user_may_intervene`], in the shape a caller that has
/// to *answer* needs: the UI hides the affordance, but the IPC behind it stays
/// reachable, and a request that arrives anyway is owed the reason rather than a
/// silent no-op. The reasons are not interchangeable — one resolves itself when
/// the run finishes, the others never will.
pub fn intervention_refusal(
    action: SyncIntervention,
    standing: SyncStanding<'_>,
) -> Option<&'static str> {
    if run_is_live(standing.feature_status) {
        return Some(
            "This run is still going and owns its own sync. \
             Wait for it to finish, or stop it first.",
        );
    }
    // Two reasons, not one: the first clears itself when the run ends and the
    // second when the turn does, and a reader told the wrong one waits on the
    // wrong thing.
    if matches!(standing.liveness, SyncLiveness::Live) {
        return Some(
            "A sync or resolution is already running on this feature and owns its \
             worktree. Wait for it to finish, or stop it first.",
        );
    }
    match standing.status {
        SyncSessionStatus::Conflicted | SyncSessionStatus::ResolutionFailed => match action {
            SyncIntervention::Abort | SyncIntervention::Resolve => None,
            SyncIntervention::Publish | SyncIntervention::Discard => {
                Some("This sync has no resolution yet — there is nothing to publish or undo.")
            }
        },
        SyncSessionStatus::Blocked => blocked_refusal(action, standing),
        // A resolution is a merge commit sitting on the feature branch. Abort
        // is refused rather than reused for it: that path undoes an *open*
        // merge and records a sync nobody published as abandoned, which for a
        // committed resolution would leave the branch carrying work the row
        // says was given up on. `Discard` is the one that moves the branch.
        SyncSessionStatus::Resolved => match action {
            SyncIntervention::Publish | SyncIntervention::Discard if !standing.published => None,
            SyncIntervention::Publish | SyncIntervention::Discard => {
                Some("This resolution is already on origin.")
            }
            SyncIntervention::Abort | SyncIntervention::Resolve => {
                Some("This sync is already resolved. Publish the resolution or discard it.")
            }
        },
        SyncSessionStatus::Resolving => {
            Some("An agent is already resolving this sync; give it time or stop the run.")
        }
        _ => Some("This sync has nothing left to act on."),
    }
}

/// The refusals a [`SyncSessionStatus::Blocked`] session owes, which are not
/// one answer but two.
///
/// [`SyncBlockedStage::Push`] is the stage where the merge is committed on the
/// feature branch and only its publication failed, and the six others are
/// stages where nothing was merged at all. Told apart, the push-blocked row can
/// offer the press that finishes what it started; collapsed, the only thing on
/// offer was a retry — which merges nothing, since the branch already has
/// `origin/<base>`, and reports `up_to_date` while the unpublished merge sits
/// where it was left.
fn blocked_refusal(action: SyncIntervention, standing: SyncStanding<'_>) -> Option<&'static str> {
    let carries_merge = standing.blocked_stage == Some(SyncBlockedStage::Push);
    match action {
        SyncIntervention::Abort => None,
        SyncIntervention::Resolve => Some(
            "This sync stopped before it reached a merge, so there are no conflicts to \
             resolve. Fix what blocked it and sync again, or abandon the sync.",
        ),
        SyncIntervention::Publish if carries_merge && !standing.published => None,
        SyncIntervention::Publish if carries_merge => Some("This merge is already on origin."),
        SyncIntervention::Discard if carries_merge => Some(
            "This sync's merge is committed on the branch. Publish it, or move the branch \
             back yourself — undoing it here would be a guess at what else has landed since.",
        ),
        SyncIntervention::Publish | SyncIntervention::Discard => {
            Some("This sync never reached a merge, so there is nothing to publish or undo.")
        }
    }
}

/// What a session becomes once origin is confirmed to hold its merge.
///
/// A resolution keeps its status: `pushed_at` is where publication is recorded
/// (V45) and the review card is selected on `resolved`, so promoting it would
/// take the state away from the reader it was held for. A `Push`-blocked sync
/// is the opposite case — nothing about it was ever conflicted, and the failed
/// push *was* the whole of the failure, so once it lands the session is the
/// clean merge it would have been.
pub fn published_status(status: SyncSessionStatus) -> SyncSessionStatus {
    match status {
        SyncSessionStatus::Blocked => SyncSessionStatus::Merged,
        other => other,
    }
}

/// Why the session already on the row may not be replaced by a fresh sync, or
/// `None` when it may.
///
/// [`SyncSessionPort::open`](crate::ports::sync_session::SyncSessionPort::open)
/// is an upsert on one row per feature, so starting a sync takes the previous
/// one's `head_before`, `merge_commit_sha` and `pushed_at` with it. For every
/// other session that is the point — the row describes the sync in flight.
///
/// A committed, unpublished resolution is the exception, and not a small one.
/// `head_before` is the only record of where the branch was and nothing can
/// recover it afterwards; the merge nobody has read becomes part of the next
/// sync's baseline on its way to origin; and because the second merge finds
/// `origin/<base>` already in the branch it changes nothing, so the row lands
/// on [`SyncSessionStatus::UpToDate`] — terminal, which `reconcile` then passes
/// through forever and every intervention refuses. Refusing the sync is what
/// keeps the resolution reachable.
pub fn resync_refusal(
    status: SyncSessionStatus,
    published: bool,
    blocked_stage: Option<SyncBlockedStage>,
) -> Option<&'static str> {
    if published {
        return None;
    }
    match status {
        SyncSessionStatus::Resolved => Some(
            "The last sync left a resolution on this branch that nobody has published or \
             discarded. Publish it or discard it first, then sync again.",
        ),
        SyncSessionStatus::Blocked if blocked_stage == Some(SyncBlockedStage::Push) => Some(
            "The last sync committed a merge on this branch and could not push it. \
             Publish it or abandon the sync first, then sync again.",
        ),
        _ => None,
    }
}

/// What happens to a resolution the moment it is committed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionPublish {
    /// Send it to origin, as every resolution did before there was a choice.
    Push,
    /// Leave it on the branch for someone to look at first.
    HoldForReview,
}

/// Whether a resolution landing now could actually be looked at by anybody.
///
/// This is the tree's existing answer to "is a human in a position to act on
/// this sync" — the same [`run_is_live`] the refusals above are built on —
/// rather than a second notion of attendedness that would have to be kept in
/// step with it. It falls out correctly for the cases that must never wait: a
/// workflow's own `sync` node only ever runs while its driver holds the
/// feature, so a headless run and a detached one both answer `false` here
/// without anything having to know which transport or which binary it is.
pub fn resolution_is_reviewable(feature_status: &str) -> bool {
    !run_is_live(feature_status)
}

/// Whether a landed resolution publishes itself or waits.
///
/// `review_before_push` is the project's setting (migration V45), where `None`
/// is "no opinion". The setting may only turn review *off*: a request to review
/// something nobody can reach is granted by holding a commit that nothing will
/// ever publish, which is worse than the push it was trying to prevent.
pub fn publish_policy(review_before_push: Option<bool>, reviewable: bool) -> ResolutionPublish {
    if reviewable && review_before_push.unwrap_or(true) {
        ResolutionPublish::HoldForReview
    } else {
        ResolutionPublish::Push
    }
}

/// Whether anything is still running this feature's sync, as an observation
/// rather than something inferred from the row.
///
/// `resolving` and `syncing` are the two statuses whose writer may still be
/// alive, and nothing a probe of the worktree can see separates a merge in
/// progress from one whose process died holding it. Correcting them without
/// this is what put Abort — `git merge --abort`, `worktree remove --force`,
/// `remove_dir_all` — in front of a directory an agent was mid-write in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncLiveness {
    /// Something is running it now, so the row it wrote stands.
    Live,
    /// Nothing is. The correction is owed, and after a restart this is what a
    /// process-local claim that died answers — which is what keeps a conflict
    /// its resolver abandoned recoverable rather than frozen.
    Gone,
}

/// The two traces a running sync leaves, combined into the one answer.
///
/// Neither covers the other, and that is why both are read. An out-of-band turn
/// — the pane's own Sync and Resolve presses — runs on a feature no driver
/// owns, so its only trace is the process-local claim; a workflow's `sync` node
/// runs under a driver that holds no entry any reader can see, and its trace is
/// the feature status. A restart empties the first and leaves the second to the
/// watchdog that owns run recovery, so neither outlives the work it describes.
pub fn sync_liveness(turn_claimed: bool, feature_status: &str) -> SyncLiveness {
    if turn_claimed || run_is_live(feature_status) {
        SyncLiveness::Live
    } else {
        SyncLiveness::Gone
    }
}

/// Feature statuses during which a driver still owns the branch.
///
/// A run parked at a gate counts: the driver is alive and will carry on through
/// its sync step the moment the gate is answered.
fn run_is_live(feature_status: &str) -> bool {
    matches!(
        feature_status,
        "pending" | "running" | "verifying" | "awaiting_gate" | "gated" | "syncing_origin"
    )
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
    /// `HEAD` has moved off the sha the sync started from, or `None` when the
    /// session recorded no starting sha and the question cannot be asked.
    ///
    /// A closed merge over a clean tree has two causes that are otherwise
    /// identical on disk — someone committed the resolution, or someone ran
    /// `git merge --abort` — and they want opposite answers.
    pub head_advanced: Option<bool>,
}

/// The stored status corrected by what is on disk, and the stage that goes
/// with it.
///
/// `blocked_stage` is `Some` only when this correction is what decided the
/// session is blocked; `None` leaves whatever the row already recorded, so a
/// `Blocked` row passed through keeps the stage its own failure named.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncVerdict {
    pub status: SyncSessionStatus,
    pub blocked_stage: Option<SyncBlockedStage>,
}

impl From<SyncSessionStatus> for SyncVerdict {
    fn from(status: SyncSessionStatus) -> Self {
        Self {
            status,
            blocked_stage: None,
        }
    }
}

/// The stored status corrected by what is on disk and by what is still running.
///
/// `probe` is `None` when the tree was not observed — either the session names
/// none to look at, or the look did not come back. Neither is the same as
/// looking and finding nothing: a sync that has not provisioned a worktree yet
/// would otherwise read as abandoned on its first poll, and a dropped
/// connection would retire a live conflict permanently.
///
/// `liveness` is the second observation and it is not interchangeable with the
/// first. A worktree with `MERGE_HEAD` set looks the same whether the process
/// that opened it is still there or died an hour ago, so the probe alone can
/// only correct the second case by also destroying the first.
pub fn reconcile(
    stored: SyncSessionStatus,
    probe: Option<&SyncWorkspaceProbe>,
    liveness: SyncLiveness,
) -> SyncVerdict {
    use SyncSessionStatus::*;

    if stored.is_terminal() {
        return stored.into();
    }
    if let Some(probe) = probe {
        if !probe.worktree_exists {
            // The tree the session was about is gone — force-removed by a later
            // sync, cleaned up by hand, or never re-created after a restart.
            // Nothing remains to resolve, continue or abort, and that holds
            // whether or not something is still nominally running: a turn whose
            // worktree has been deleted under it is not going to finish.
            //
            // A resolution is the exception, and it is not a small one: its
            // commit is on the *feature branch*, not in the throwaway tree the
            // merge ran in, so losing that directory loses nothing at all.
            // Retiring it as `Aborted` would tell the reader the merge is gone
            // while it is sitting on their branch, unpublished — and `Aborted`
            // is terminal, so nothing would ever revisit it to say otherwise.
            return if matches!(stored, Resolved) {
                Resolved.into()
            } else {
                Aborted.into()
            };
        }
    }
    // The two statuses a live writer is still entitled to. Their rows are not
    // claims to be checked while the thing that wrote them is running; they are
    // the only report of it there is.
    if matches!(stored, Syncing | Resolving) && matches!(liveness, SyncLiveness::Live) {
        return stored.into();
    }
    if matches!(stored, Syncing) {
        // A merge nobody is running never reached a verdict, and what its tree
        // holds is unknown by construction — possibly clean, possibly
        // half-applied. `Blocked` is that fact: it offers the abort that
        // reclaims the directory and withholds the resolver, which would go
        // looking for conflicts nothing has established are there. Reading it
        // as a conflict is the mistake `sync_failure::merge_failure_stage`
        // refuses at the other end of the same sync.
        return SyncVerdict {
            status: Blocked,
            blocked_stage: Some(SyncBlockedStage::Merge),
        };
    }
    let Some(probe) = probe else {
        return stored.into();
    };
    match stored {
        // Reached only once `liveness` has said nothing is resolving, so the
        // open merge is a conflict waiting for somebody rather than work in
        // progress.
        Resolving if probe.merge_in_progress => Conflicted.into(),
        Resolved if probe.merge_in_progress => Conflicted.into(),
        // The merge is closed over a clean tree. Two things do that and they
        // want opposite answers: a resolution somebody committed — an agent
        // that staged on its own, or the user in their own editor — and a
        // `git merge --abort` run by hand. Only the commit moves `HEAD` off the
        // sha the sync started from, so that is what separates them; without a
        // starting sha to compare against, neither answer is earned and the
        // stored status stands.
        //
        // `Blocked` is deliberately not in this arm: a push that failed leaves
        // exactly this shape, and nothing about it was ever conflicted.
        Conflicted | Resolving | ResolutionFailed if !probe.merge_in_progress && !probe.dirty => {
            match probe.head_advanced {
                Some(true) => Resolved.into(),
                Some(false) => Aborted.into(),
                None => stored.into(),
            }
        }
        other => other.into(),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/sync_session.rs"]
mod tests;
