//! Reading `git worktree list --porcelain` into the primary checkout and the
//! linked worktrees beside it.
//!
//! Two adapters had grown their own state machine over this one output —
//! `list_worktrees` and `merge_subtask`, in
//! `crates/demeteo-core/src/adapters/worktree/git_ops/` — disagreeing on the
//! only question that matters here: `list_worktrees` threw entry 0 away, while
//! `merge_subtask` kept it deliberately, because the feature branch it hunts
//! for is often checked out in the main repo. Neither copy was reachable from a
//! test without standing up the transport underneath it. See [`crate::domain`].
//!
//! Which entry is the primary is load-bearing in a way the discarding copy hid.
//! Three callers of `list_worktrees` filter its result and then run
//! `worktree remove --force` and `rm -rf` over whatever survives, so handing
//! them the main checkout puts it one filter bug away from deletion. Keeping
//! the split in the return type — rather than in a `bool` on each entry, or an
//! index convention every caller has to remember — is what makes "the primary
//! never reaches those callers" something the compiler helps with.
//!
//! The `git` invocation stays with the adapters that own the transport; this
//! module never sees a machine.

use crate::domain::models::WorktreeInfo;

/// One `git worktree list --porcelain` reading, split by the only distinction
/// its callers make.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct WorktreeListing {
    /// Porcelain entry 0. Git emits the primary checkout first, always, and
    /// reports it as the path it resolved — symlinks already followed. That
    /// makes it the one physical path available without asking the target host
    /// a second question.
    pub primary: Option<WorktreeInfo>,
    /// Entries 1..n, in the order git reported them.
    pub linked: Vec<WorktreeInfo>,
}

impl WorktreeListing {
    /// Every entry, primary first, in git's order.
    pub fn all(&self) -> impl Iterator<Item = &WorktreeInfo> {
        self.primary.iter().chain(self.linked.iter())
    }
}

/// Walk the porcelain and split entry 0 from the rest.
///
/// Paths are taken verbatim. An earlier copy trimmed them, which silently
/// corrupts a worktree whose directory name genuinely ends in whitespace;
/// [`str::lines`] already drops the `\r` that trimming was really there for.
pub fn parse(porcelain: &str) -> WorktreeListing {
    let mut listing = WorktreeListing::default();
    let mut path: Option<String> = None;
    let mut branch: Option<String> = None;
    let mut is_locked = false;

    let mut flush = |path: Option<String>, branch: Option<String>, is_locked: bool| {
        let Some(path) = path else {
            return;
        };
        let entry = WorktreeInfo {
            path,
            branch,
            is_locked,
        };
        if listing.primary.is_none() {
            listing.primary = Some(entry);
        } else {
            listing.linked.push(entry);
        }
    };

    for line in porcelain.lines() {
        if let Some(rest) = line.strip_prefix("worktree ") {
            flush(path.take(), branch.take(), is_locked);
            path = Some(rest.to_string());
            is_locked = false;
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.strip_prefix("refs/heads/").unwrap_or(rest).to_string());
        } else if line.starts_with("locked") {
            is_locked = true;
        }
    }
    flush(path, branch, is_locked);

    listing
}

#[cfg(test)]
#[path = "../../tests/domain/worktree_listing.rs"]
mod tests;
