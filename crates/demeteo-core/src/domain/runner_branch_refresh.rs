//! Bringing a `demeteo-runner`'s clone of a run's branch up to date after a
//! desktop sync published to it. See [`crate::domain`].
//!
//! A sync of a detached run happens in the desktop's clone and is pushed from
//! there, and nothing else ever tells the runner: it fetches only when it
//! bootstraps, and a retry or replay starts from its own `refs/heads/<branch>`.
//! Its next terminal push then carried the pre-sync branch back to origin.
//! [`push_lease`](crate::domain::push_lease) turns that push into a refusal;
//! this is the half that makes it unnecessary, by moving the runner's branch —
//! and its remote-tracking ref, which the lease reads — to what origin holds.
//!
//! It only ever fast-forwards. A runner branch origin does not contain is work
//! the runner has not pushed, and a checked-out branch with local changes is a
//! tree somebody may be reading; both are refused rather than reset.

use serde::{Deserialize, Serialize};

use crate::domain::sync_session::SyncSessionStatus;

/// The runner RPC this module decides for.
pub const METHOD: &str = "refresh_feature_branch";

/// Run-row statuses after which nothing on the runner touches the branch until
/// a person rewinds the run. `needs-credentials` is not one: the terminal push
/// resumes from it the moment credentials arrive.
const SETTLED: [&str; 7] = [
    "completed",
    "awaiting_mr",
    "pr_ready",
    "failed",
    "interrupted",
    "cancelled",
    "over-budget",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshRefusal {
    /// The run row is not settled, so its driver or its terminal push may move
    /// the branch underneath the refresh.
    RunActive {
        status: String,
    },
    NotOnOrigin,
    NoLocalBranch,
    UnpushedWork {
        local: String,
        origin: String,
    },
    DirtyCheckout {
        worktree: String,
    },
}

impl std::fmt::Display for RefreshRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RunActive { status } => write!(
                f,
                "the run is still {status} on the runner, which may move the branch itself"
            ),
            Self::NotOnOrigin => f.write_str("origin has no such branch"),
            Self::NoLocalBranch => f.write_str("the runner holds no copy of the branch"),
            Self::UnpushedWork { local, origin } => write!(
                f,
                "the runner's branch ({local}) has commits origin ({origin}) does not; \
                 they were never pushed"
            ),
            Self::DirtyCheckout { worktree } => write!(
                f,
                "the branch is checked out at {worktree} with uncommitted changes"
            ),
        }
    }
}

/// `None` when a run in `run_status` may have its branch moved.
pub fn refusal_for_run(run_status: &str) -> Option<RefreshRefusal> {
    (!SETTLED.contains(&run_status)).then(|| RefreshRefusal::RunActive {
        status: run_status.to_string(),
    })
}

/// The worktree holding the branch, when one does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkout<'a> {
    pub worktree: &'a str,
    pub dirty: bool,
}

/// What the runner observed in its clone, after fetching origin's branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchFacts<'a> {
    pub local_tip: Option<&'a str>,
    /// `None` when origin has no such branch.
    pub origin_tip: Option<&'a str>,
    /// Whether `local_tip` is an ancestor of `origin_tip`. Read only when both
    /// exist and differ.
    pub local_in_origin: bool,
    pub checkout: Option<Checkout<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshPlan {
    UpToDate {
        tip: String,
    },
    /// `merge --ff-only` in the worktree that has the branch checked out, so
    /// its index and files move with the ref.
    FastForwardCheckout {
        worktree: String,
        from: String,
        to: String,
    },
    /// `update-ref` with `from` as the expected old value.
    MoveRef {
        from: String,
        to: String,
    },
    Refuse(RefreshRefusal),
}

pub fn plan_refresh(run_status: &str, facts: &BranchFacts<'_>) -> RefreshPlan {
    if let Some(refusal) = refusal_for_run(run_status) {
        return RefreshPlan::Refuse(refusal);
    }
    let Some(origin) = facts.origin_tip else {
        return RefreshPlan::Refuse(RefreshRefusal::NotOnOrigin);
    };
    let Some(local) = facts.local_tip else {
        return RefreshPlan::Refuse(RefreshRefusal::NoLocalBranch);
    };
    if local == origin {
        return RefreshPlan::UpToDate {
            tip: origin.to_string(),
        };
    }
    if !facts.local_in_origin {
        return RefreshPlan::Refuse(RefreshRefusal::UnpushedWork {
            local: local.to_string(),
            origin: origin.to_string(),
        });
    }
    match &facts.checkout {
        Some(Checkout {
            worktree,
            dirty: true,
        }) => RefreshPlan::Refuse(RefreshRefusal::DirtyCheckout {
            worktree: worktree.to_string(),
        }),
        Some(Checkout {
            worktree,
            dirty: false,
        }) => RefreshPlan::FastForwardCheckout {
            worktree: worktree.to_string(),
            from: local.to_string(),
            to: origin.to_string(),
        },
        None => RefreshPlan::MoveRef {
            from: local.to_string(),
            to: origin.to_string(),
        },
    }
}

/// The RPC's answer. A refusal is an answer, not an error: the runner looked
/// and decided, and the caller reports it the same way whatever the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BranchRefreshOutcome {
    Updated { from: String, to: String },
    UpToDate { tip: String },
    Refused { reason: String },
}

/// The path of the worktree that has `branch` checked out, from
/// `git worktree list --porcelain`.
pub fn checkout_holding(porcelain: &str, branch: &str) -> Option<String> {
    crate::domain::worktree_listing::parse(porcelain)
        .all()
        .find(|w| w.branch.as_deref() == Some(branch))
        .map(|w| w.path.clone())
}

/// Whether a failed `git fetch origin <branch>` failed because origin has no
/// such branch, which is an observation rather than a failure.
pub fn is_missing_remote_ref(error: &str) -> bool {
    error.contains("couldn't find remote ref")
}

/// The part of a sync session that says whether, and with what, it reached
/// origin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncMark {
    pub status: SyncSessionStatus,
    pub merge_commit_sha: Option<String>,
    pub pushed_at: Option<i64>,
}

/// Whether the sync that ran between `before` and `after` put something on
/// origin. A clean merge is pushed as part of the sync and never records
/// `pushed_at`, so `Merged` counts on its own.
pub fn newly_on_origin(before: Option<&SyncMark>, after: Option<&SyncMark>) -> bool {
    let Some(after) = after else {
        return false;
    };
    let on_origin = after.pushed_at.is_some() || after.status == SyncSessionStatus::Merged;
    on_origin && before != Some(after)
}

/// What the user is told about a refresh, or `None` when there is nothing to
/// tell. The sync itself has already succeeded either way.
pub fn refresh_notice(
    branch: &str,
    result: &Result<BranchRefreshOutcome, String>,
) -> Option<String> {
    let lead = format!("The sync reached origin/{branch}, but the runner's copy of the branch");
    match result {
        Ok(BranchRefreshOutcome::Updated { .. } | BranchRefreshOutcome::UpToDate { .. }) => None,
        Ok(BranchRefreshOutcome::Refused { reason }) => Some(format!(
            "{lead} was not brought up to date: {reason}. A later push of this run from the \
             runner will be refused rather than overwrite the sync."
        )),
        Err(error) if error.starts_with("unknown method") => Some(format!(
            "{lead} was not brought up to date: this runner is too old to be told. Upgrade it \
             before retrying or replaying the run, or its next push will overwrite the sync."
        )),
        Err(error) => Some(format!(
            "{lead} could not be brought up to date: {error}. Sync again once the runner answers, \
             before retrying or replaying the run."
        )),
    }
}

#[cfg(test)]
#[path = "../../tests/domain/runner_branch_refresh.rs"]
mod tests;
