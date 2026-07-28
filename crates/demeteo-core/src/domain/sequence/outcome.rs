//! How a `sequence` step fails, and what it owes the user when it does.

use std::fmt;

/// Why a `sequence` step could not finish.
///
/// Replaces the `(String, bool)` the step's internals used to pass around,
/// where the `bool` meant *environmental*. That encoding had two costs. A
/// transposed or misread flag silently moved a failure between retry
/// budgets — an environmental failure must not consume the `on_failure`
/// allowance, and an implementation failure must. And cancellation had no
/// representation at all: it travelled as the magic string
/// `"Execution cancelled by user"` in the `Failed` position, which nothing
/// matched on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequenceError {
    /// The user asked to stop. Not a failure — the step reports
    /// `interrupted`, and its branch rolls back.
    Cancelled,
    /// The implementation is wrong: a task failed, a merge could not be
    /// made to land, the aggregate budget ran out. Editing source can fix
    /// it, so it consumes the retry budget.
    Failed(String),
    /// The environment broke: an agent could not spawn, a worktree
    /// vanished, a machine is unreachable. Redirecting to an
    /// implementation step cannot fix these, so they must not consume the
    /// retry budget.
    Environmental(String),
}

impl SequenceError {
    /// Prefix the message with the context that produced it.
    ///
    /// [`Self::Cancelled`] passes through untouched: it carries no message,
    /// and a cancel is not *about* whichever task happened to notice it.
    pub fn with_context(self, ctx: impl fmt::Display) -> Self {
        match self {
            Self::Cancelled => Self::Cancelled,
            Self::Failed(msg) => Self::Failed(format!("{ctx}: {msg}")),
            Self::Environmental(msg) => Self::Environmental(format!("{ctx}: {msg}")),
        }
    }

    /// The message to store and show, or `None` for a cancellation.
    pub fn message(&self) -> Option<&str> {
        match self {
            Self::Cancelled => None,
            Self::Failed(msg) | Self::Environmental(msg) => Some(msg),
        }
    }
}

impl fmt::Display for SequenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => f.write_str("Execution cancelled by user"),
            Self::Failed(msg) | Self::Environmental(msg) => f.write_str(msg),
        }
    }
}

/// What happened to the feature branch on the way out of a failed sequence
/// step. Folded into the stored error message, because the user has to know
/// the branch's state before they retry or ship — each variant leaves it
/// somewhere different.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureDisposition {
    /// The branch was reset to its pre-attempt tip; a retry starts clean.
    RolledBack,
    /// The reset failed (usually an unremovable worktree) and the failed
    /// attempt's commits are still on the branch.
    RollbackFailed,
    /// The tasks that completed before the failure were merged to the
    /// feature branch; the retry resumes from the failed task.
    PrefixLanded { landed: usize, total: usize },
}

impl FailureDisposition {
    pub fn from_rollback(rolled_back: bool) -> Self {
        if rolled_back {
            Self::RolledBack
        } else {
            Self::RollbackFailed
        }
    }

    /// Fold the branch's state into the failure message. A rollback that did
    /// not happen leaves the failed attempt's commits on the feature branch,
    /// and the user has to know that before they retry or ship — claiming a
    /// clean slate we did not deliver is worse than the failure itself. A
    /// checkpointed prefix is the deliberate version of the same situation:
    /// commits on the branch, but kept on purpose and resumed from on retry.
    pub fn decorate(&self, msg: &str, branch: &str) -> String {
        match self {
            Self::RolledBack => format!(
                "{} (the step's task commits have been rolled back for a clean retry)",
                msg
            ),
            Self::RollbackFailed => format!(
                "{} (WARNING: the step's task commits could NOT be rolled back and are still on \
                 branch '{}' — its worktree could not be removed. Inspect the branch before \
                 retrying.)",
                msg, branch
            ),
            Self::PrefixLanded { landed, total } => format!(
                "{} ({} of {} tasks completed before the failure; their commits were kept and \
                 merged to branch '{}', and a retry will resume from the failed task instead of \
                 starting over)",
                msg, landed, total, branch
            ),
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/sequence/outcome.rs"]
mod tests;
