//! Reading `git status --porcelain` for the files a merge left unresolved.
//!
//! Which two-letter XY codes mean "conflicted", and what the UI calls each, is
//! a decision — and it was written out three times: once in the step
//! executor's sync flow, once in its conflict pass, and once in
//! `adapters/worktree/git_ops`. Not one copy had a test.
//!
//! It is also [`ExecutionPort`](crate::ports::execution::ExecutionPort)-
//! observed output, parsed identically on the local and the SSH path. Three
//! copies of one parser over one contract is precisely the shape the parity
//! invariant exists to keep from drifting, so there is now one.
//!
//! The `git status` invocation itself stays in the adapters that own the
//! transport; this module never sees a machine.

use crate::domain::models::ConflictFile;

/// Walk `git status --porcelain` and pull out the unmerged paths.
///
/// The `kind` strings cross to the frontend through
/// [`SyncOutcomeView`](crate::ports::step_executor::SyncOutcomeView) — they
/// are wire values, not labels, and renaming one is a frontend change.
pub(crate) fn parse_unmerged(porcelain: &str) -> Vec<ConflictFile> {
    porcelain
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if line.len() < 3 {
                return None;
            }
            let xy = &line[..2];
            let path = line[3..].trim().to_string();
            let kind = match xy {
                "UU" | "AA" | "DD" => "both-modified".to_string(),
                "UA" => "added-by-them".to_string(),
                "AU" => "added-by-us".to_string(),
                "UD" => "deleted-by-them".to_string(),
                "DU" => "deleted-by-us".to_string(),
                _ => return None,
            };
            Some(ConflictFile { path, kind })
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/domain/merge_status.rs"]
mod tests;
