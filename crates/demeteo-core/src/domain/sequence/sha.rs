//! A resolved commit SHA, told apart from the strings it travels beside.
//!
//! A `sequence` step passes git a lot of bare `&str`s, and only some of them
//! are commits. `branch_force(repo, branch, sha)`, `merge_base(repo, a, b)`,
//! `update_ref(repo, git_ref, sha)` — every one of those has at least one
//! operand that is a *name* and at least one that is a commit, in adjacent
//! positions, with the same type. Swapping a pair compiles, and what it
//! produces is a `reset --hard` onto the wrong thing rather than an error.
//!
//! So the commits get their own type. It is deliberately thin: no validation,
//! no hex check, no length rule. `git rev-parse` is the authority on what a
//! SHA is, and a type that second-guessed it would reject the abbreviated and
//! peeled forms git itself accepts. What this buys is only that a SHA cannot
//! be passed where a branch name belongs — which is the mistake that was
//! actually available.

use std::fmt;

/// A commit SHA as git resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sha(String);

impl Sha {
    /// A SHA from somewhere other than a fresh `rev-parse`: the checkpoint
    /// row read back from the database, or a test's fixture.
    pub fn new(sha: impl Into<String>) -> Self {
        Self(sha.into())
    }

    /// `git rev-parse`'s stdout.
    ///
    /// Trimmed here rather than by each caller, which is what every one of
    /// them already did — git prints a trailing newline, and a SHA carrying
    /// it interpolates into the next command as a two-line operand.
    ///
    /// **Empty stays empty.** `rev-parse` can succeed with nothing to say,
    /// and the callers do not agree on what that means: the task loop treats
    /// it as "this task is not checkpointable", while the rollback anchor
    /// takes it as-is. Rejecting it here would move that decision.
    pub fn from_output(stdout: &str) -> Self {
        Self(stdout.trim().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Did git answer with nothing? See [`Self::from_output`].
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Sha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/sha.rs"]
mod tests;
