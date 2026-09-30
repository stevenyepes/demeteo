//! Which leaked siblings of a project's clone a sweep may reclaim. See
//! [`crate::domain`].
//!
//! Every feature leaves a [`feature_cache_dir`](crate::paths::feature_cache_dir)
//! beside the clone and every step a `{clone}_wt_{id}` worktree. Releasing them
//! when a feature ends is the forward path's job; this is the net under it, and
//! what reclaims whatever that path missed before it existed.
//!
//! The asymmetry between the two kinds is deliberate. A cache is named after a
//! *branch*, so a feature row can vouch for it — but only through the
//! `branch_prefix` setting as it reads today, and a changed prefix makes a live
//! feature's cache look unclaimed. An unclaimed cache is therefore reported,
//! never deleted. A worktree is named after a step, which no row records, but
//! git does: a registered worktree is in use, and an unregistered one is debris
//! once it is old enough that no `git worktree add` can still be on its way.
//!
//! The default branch's cache belongs to no feature: every Ask thread and
//! Discovery of the project reads through it. It goes only when none of them
//! holds a worktree — by its row, by a turn in flight, or by git's own list —
//! and none has been touched within the project's idle TTL. Provisioning
//! reseeds it on the next session exactly as it reseeds a feature's, so a
//! release costs that session one cold copy and nothing else.

use std::collections::HashSet;

use serde::Serialize;

use crate::domain::cache_release::{cache_releasable, cache_releasable_at, idle_past_ttl};

/// How long an unregistered worktree directory is left alone.
///
/// Provisioning clears the path and then `git worktree add`s it, so the only
/// unregistered-but-wanted directory is one inside that window, which is
/// seconds; an hour is far past it and still reclaims within one session.
pub const WORKTREE_GRACE_SECS: u64 = 60 * 60;

/// The worktree id an Ask thread's checkout is provisioned under, before the
/// thread's id.
pub const ASK_WORKTREE_PREFIX: &str = "ask-";
/// The same for a Discovery's.
pub const DISCOVERY_WORKTREE_PREFIX: &str = "discovery-";

/// One entry of the directory the clone sits in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sibling {
    pub name: String,
    pub is_dir: bool,
    /// Seconds since the epoch, on the clock of the host that holds it.
    pub modified_secs: Option<u64>,
}

/// A feature of the project, as far as its cache is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownFeature {
    pub branch: String,
    pub status: String,
    pub mr_state: Option<String>,
    /// Milliseconds since the epoch.
    pub last_activity_ms: i64,
}

/// What the project's Ask threads and Discoveries say about the default
/// branch's cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionActivity {
    /// A row still names a worktree, or a turn is running.
    pub holding_worktree: bool,
    /// The newest session's last touch, in milliseconds; `None` when the
    /// project has had none.
    pub last_activity_ms: Option<i64>,
}

/// Everything a sweep of one clone's siblings decides from.
#[derive(Debug, Clone)]
pub struct Observation<'a> {
    /// The clone directory's own name — the prefix every sibling carries.
    pub clone_name: &'a str,
    /// The names of the project's other clones, which share the directory.
    pub other_repos: &'a [String],
    pub siblings: &'a [Sibling],
    /// The [`path_basename`] of every path `git worktree list` reported, or
    /// `None` when git could not be asked.
    pub registered: Option<&'a HashSet<String>>,
    pub features: &'a [KnownFeature],
    /// Ask and discovery sessions share this branch's cache.
    pub default_branch: &'a str,
    /// `None` when the sessions could not be read.
    pub sessions: Option<SessionActivity>,
    /// Already resolved by
    /// [`cache_idle_ttl_days`](crate::domain::cache_release::cache_idle_ttl_days).
    pub cache_idle_ttl_days: u32,
    pub now_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SiblingKind {
    Cache,
    Worktree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Delete,
    Keep,
    /// Not provably anyone's, and not provably no one's: reported for a human.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    FeatureReleased,
    /// Every feature on the branch is idle past the TTL, or already released.
    FeatureIdle,
    FeatureMayRun,
    /// The project's idle TTL is `0`.
    DefaultBranchCache,
    SessionHoldsWorktree,
    SessionsRecent,
    SessionsIdle,
    SessionsUnknown,
    NoKnownFeature,
    Registered,
    /// Without git's list, a live worktree and a leaked one look identical.
    RegistrationUnknown,
    WithinGrace,
    AgeUnknown,
    Unregistered,
    OtherRepository,
}

impl Reason {
    /// Why, in words a sweep report can print beside the path.
    pub fn describe(self) -> &'static str {
        match self {
            Self::FeatureReleased => "every feature on its branch is merged, closed, archived or deleted",
            Self::FeatureIdle => "every feature on its branch has ended or sat idle past the project's cache TTL",
            Self::FeatureMayRun => "a feature on its branch can still run",
            Self::DefaultBranchCache => "the default branch's cache, and the project's cache TTL is off",
            Self::SessionHoldsWorktree => "an Ask or Discovery session still holds a worktree",
            Self::SessionsRecent => "an Ask or Discovery session was active within the project's cache TTL",
            Self::SessionsIdle => "no Ask or Discovery session holds a worktree or has been active within the project's cache TTL",
            Self::SessionsUnknown => "the project's Ask and Discovery sessions could not be read",
            Self::NoKnownFeature => "no feature claims it",
            Self::Registered => "git lists it as a worktree",
            Self::RegistrationUnknown => "git could not list the worktrees",
            Self::WithinGrace => "too recent to be debris",
            Self::AgeUnknown => "its age could not be read",
            Self::Unregistered => "git does not list it and it is past the grace period",
            Self::OtherRepository => "it belongs to another repository of the project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SweepAction {
    pub name: String,
    pub kind: SiblingKind,
    pub verdict: Verdict,
    pub reason: Reason,
}

/// A verdict for every directory named as one of the clone's caches or
/// worktrees. Anything else in the directory is not mentioned at all.
pub fn plan(obs: &Observation<'_>) -> Vec<SweepAction> {
    obs.siblings
        .iter()
        .filter(|s| s.is_dir)
        .filter_map(|s| {
            let kind = kind_of(obs.clone_name, &s.name)?;
            let (verdict, reason) = if claimed_by_other_repo(obs.other_repos, &s.name) {
                (Verdict::Keep, Reason::OtherRepository)
            } else {
                match kind {
                    SiblingKind::Cache => cache_verdict(obs, s),
                    SiblingKind::Worktree => worktree_verdict(obs, s),
                }
            };
            Some(SweepAction {
                name: s.name.clone(),
                kind,
                verdict,
                reason,
            })
        })
        .collect()
}

/// The last component of a path reported by any host.
///
/// Split on both separators rather than through [`std::path::Path`], which
/// only knows the separators of the host it runs on: git on Windows reports
/// `C:/…`, and a remote clone is POSIX whatever the desktop runs.
pub fn path_basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed)
}

fn kind_of(clone_name: &str, name: &str) -> Option<SiblingKind> {
    let rest = name.strip_prefix(clone_name)?;
    if rest.strip_prefix("_cache_").is_some_and(|s| !s.is_empty()) {
        Some(SiblingKind::Cache)
    } else if rest.strip_prefix("_wt_").is_some_and(|s| !s.is_empty()) {
        Some(SiblingKind::Worktree)
    } else {
        None
    }
}

fn claimed_by_other_repo(other_repos: &[String], name: &str) -> bool {
    other_repos
        .iter()
        .any(|repo| name == repo || kind_of(repo, name).is_some())
}

fn cache_name(clone_name: &str, branch: &str) -> String {
    crate::paths::feature_cache_dir(clone_name, branch)
}

fn cache_verdict(obs: &Observation<'_>, sibling: &Sibling) -> (Verdict, Reason) {
    let name = sibling.name.as_str();
    if name == cache_name(obs.clone_name, obs.default_branch) {
        return default_branch_cache_verdict(obs, sibling);
    }
    // Two features can share a branch, so one that can still run keeps it.
    let mut owners = obs
        .features
        .iter()
        .filter(|f| cache_name(obs.clone_name, &f.branch) == name)
        .peekable();
    if owners.peek().is_none() {
        return (Verdict::Unknown, Reason::NoKnownFeature);
    }
    let owners: Vec<&KnownFeature> = owners.collect();
    let now_ms = now_ms(obs);
    if owners
        .iter()
        .all(|f| cache_releasable(&f.status, f.mr_state.as_deref()))
    {
        (Verdict::Delete, Reason::FeatureReleased)
    } else if owners.iter().all(|f| {
        cache_releasable_at(
            &f.status,
            f.mr_state.as_deref(),
            f.last_activity_ms,
            now_ms,
            obs.cache_idle_ttl_days,
        )
    }) {
        (Verdict::Delete, Reason::FeatureIdle)
    } else {
        (Verdict::Keep, Reason::FeatureMayRun)
    }
}

fn default_branch_cache_verdict(obs: &Observation<'_>, sibling: &Sibling) -> (Verdict, Reason) {
    if obs.cache_idle_ttl_days == 0 {
        return (Verdict::Keep, Reason::DefaultBranchCache);
    }
    let Some(sessions) = obs.sessions else {
        return (Verdict::Keep, Reason::SessionsUnknown);
    };
    let Some(registered) = obs.registered else {
        return (Verdict::Keep, Reason::RegistrationUnknown);
    };
    if sessions.holding_worktree
        || registered
            .iter()
            .any(|name| is_session_worktree(obs.clone_name, name))
    {
        return (Verdict::Keep, Reason::SessionHoldsWorktree);
    }
    // The directory's own mtime stands in for a session row deleted since.
    let dir_ms = sibling
        .modified_secs
        .and_then(|s| i64::try_from(s.saturating_mul(1000)).ok());
    let Some(last) = sessions.last_activity_ms.max(dir_ms) else {
        return (Verdict::Keep, Reason::AgeUnknown);
    };
    if idle_past_ttl(last, now_ms(obs), obs.cache_idle_ttl_days) {
        (Verdict::Delete, Reason::SessionsIdle)
    } else {
        (Verdict::Keep, Reason::SessionsRecent)
    }
}

fn is_session_worktree(clone_name: &str, name: &str) -> bool {
    name.strip_prefix(clone_name)
        .and_then(|rest| rest.strip_prefix("_wt_"))
        .is_some_and(|id| {
            id.starts_with(ASK_WORKTREE_PREFIX) || id.starts_with(DISCOVERY_WORKTREE_PREFIX)
        })
}

fn now_ms(obs: &Observation<'_>) -> i64 {
    i64::try_from(obs.now_secs.saturating_mul(1000)).unwrap_or(i64::MAX)
}

fn worktree_verdict(obs: &Observation<'_>, sibling: &Sibling) -> (Verdict, Reason) {
    let Some(registered) = obs.registered else {
        return (Verdict::Keep, Reason::RegistrationUnknown);
    };
    if registered.contains(&sibling.name) {
        return (Verdict::Keep, Reason::Registered);
    }
    let Some(modified) = sibling.modified_secs else {
        return (Verdict::Keep, Reason::AgeUnknown);
    };
    // A host clock ahead of ours reads as a future mtime: age zero, kept.
    if obs.now_secs.saturating_sub(modified) < WORKTREE_GRACE_SECS {
        (Verdict::Keep, Reason::WithinGrace)
    } else {
        (Verdict::Delete, Reason::Unregistered)
    }
}

#[cfg(test)]
#[path = "../../tests/domain/cache_sweep.rs"]
mod tests;
