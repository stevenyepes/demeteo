//! Which branches a new terminal worktree may be cut from, read out of
//! `git for-each-ref`. See [`crate::domain`].
//!
//! One name, not two lists. Git keeps `refs/heads/main` and
//! `refs/remotes/origin/main` as separate refs, but a person choosing a base
//! is choosing *main* — offering both spellings makes the one decision that
//! matters here (is this base current with origin?) look like a naming
//! question. So the two refs collapse into one candidate carrying where it was
//! seen, and the caller cuts from `origin/<name>` whenever
//! [`BranchOption::has_remote`] holds.

use crate::domain::ids::SUBTASK_BRANCH_INFIX;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A branch a terminal worktree may be based on, and where it exists.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchOption {
    /// The branch name with no `refs/` prefix and no remote qualifier.
    pub name: String,
    /// A `refs/heads/<name>` exists.
    pub has_local: bool,
    /// A `refs/remotes/origin/<name>` exists — so a fetch can refresh it, and
    /// a base cut from it is as current as that fetch.
    pub has_remote: bool,
}

/// Read full ref names into base candidates, alphabetically.
///
/// Alphabetical rather than git's order: `for-each-ref` sorts by *refname*, so
/// every `refs/heads/*` would arrive before every `refs/remotes/*` and the
/// merged list would come out in the order the local branches happened to be
/// created. Sorting on the collapsed name is what makes the listing stable
/// across two repositories holding the same branches.
///
/// Two refs are dropped rather than shown. `origin/HEAD` is a symbolic
/// pointer, not a branch someone means to name. Subtask branches carry
/// [`SUBTASK_BRANCH_INFIX`] and belong to a pipeline run that may be mid-write
/// in a worktree of its own; offering one as a base invites cutting from a tree
/// an agent is still moving under.
pub fn parse(refs: &str) -> Vec<BranchOption> {
    let mut seen: BTreeMap<&str, (bool, bool)> = BTreeMap::new();

    for line in refs.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let (name, remote) = if let Some(local) = line.strip_prefix("refs/heads/") {
            (local, false)
        } else if let Some(remote) = line.strip_prefix("refs/remotes/origin/") {
            (remote, true)
        } else {
            continue;
        };
        if name.is_empty() || name == "HEAD" || name.contains(SUBTASK_BRANCH_INFIX) {
            continue;
        }
        let entry = seen.entry(name).or_insert((false, false));
        if remote {
            entry.1 = true;
        } else {
            entry.0 = true;
        }
    }

    seen.into_iter()
        .map(|(name, (has_local, has_remote))| BranchOption {
            name: name.to_string(),
            has_local,
            has_remote,
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/branch_listing.rs"]
mod tests;
