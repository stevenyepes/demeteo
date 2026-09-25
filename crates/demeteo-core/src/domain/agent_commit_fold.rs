//! Commits an agent made itself, and why Demeteo unmakes them.
//!
//! Demeteo owns every commit on a subtask branch: its commit is the one that
//! applies the `commit_artifacts` pathspec exclusion, carries Demeteo's
//! identity, and bears the message finalize squashes. An agent that runs
//! `git commit` bypasses all three — it commits `artifacts/…` reports the
//! exclusion exists to keep out, under the user's own identity, and leaves
//! Demeteo's commit empty. The now-tracked stale report then misleads every
//! later validate turn that reads the branch.
//!
//! So the adapter pins HEAD and its branch before the turn and, before
//! staging, rewinds to that pin with `reset --mixed`, which leaves the
//! working tree — the source of truth — to flow through Demeteo's normal
//! staging. `--soft` would not do: it leaves `artifacts/…` in the index, and
//! `add -A -- ':!artifacts'` only declines to *add* the excluded path, it does
//! not unstage it.

use serde::{Deserialize, Serialize};

/// The line every agent is told. The fold makes a commit harmless; telling
/// the agent not to spares it the turn it would spend writing one.
pub(crate) const COMMIT_OWNERSHIP_RULE: &str = "Demeteo commits your work when your turn \
     ends. Do not run `git commit` yourself, and do not amend, rebase, reset, or switch \
     branches.";

/// Where a worktree's HEAD stood at one moment. Either half is `None` when
/// it could not be read — or, for `branch`, when HEAD was detached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HeadState {
    pub sha: Option<String>,
    /// The full ref HEAD points at (`refs/heads/…`).
    pub branch: Option<String>,
}

/// One commit the agent made, as `git log` reported it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCommit {
    pub sha: String,
    /// `Name <email>` — the identity the agent's commit carried.
    pub author: String,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldPlan {
    /// HEAD and its branch are where the turn found them.
    Clean,
    /// Point HEAD back at `branch`, then `reset --mixed` to `reset_to`.
    Fold { branch: String, reset_to: String },
    /// HEAD moved but there is no pin to return to. The caller commits the
    /// worktree as it stands, exactly as it did before folding existed.
    CannotFold { reason: String },
}

/// What happened, for the caller to report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldOutcome {
    Clean,
    Folded { commits: Vec<AgentCommit> },
    CannotFold { reason: String },
}

pub fn plan_fold(pre: &HeadState, post: &HeadState) -> FoldPlan {
    if pre == post {
        return FoldPlan::Clean;
    }
    let Some(reset_to) = pre.sha.as_deref().filter(|s| !s.is_empty()) else {
        return FoldPlan::CannotFold {
            reason: "the worktree's HEAD before the turn is unknown".to_string(),
        };
    };
    let Some(branch) = pre.branch.as_deref().filter(|s| !s.is_empty()) else {
        return FoldPlan::CannotFold {
            reason: "the worktree was on a detached HEAD before the turn".to_string(),
        };
    };
    if post.sha.as_deref().is_none_or(str::is_empty) {
        return FoldPlan::CannotFold {
            reason: "the worktree's HEAD could not be read after the turn".to_string(),
        };
    }
    FoldPlan::Fold {
        branch: branch.to_string(),
        reset_to: reset_to.to_string(),
    }
}

/// The `--format` [`parse_agent_commits`] reads: sha, author, subject,
/// separated by the unit separator no subject line can contain.
pub const AGENT_COMMIT_LOG_FORMAT: &str = "%H%x1f%an <%ae>%x1f%s";

pub fn parse_agent_commits(log: &str) -> Vec<AgentCommit> {
    log.lines()
        .filter_map(|line| {
            let mut parts = line.splitn(3, '\u{1f}');
            let sha = parts.next()?.trim();
            let author = parts.next()?;
            let subject = parts.next().unwrap_or("");
            (!sha.is_empty()).then(|| AgentCommit {
                sha: sha.to_string(),
                author: author.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect()
}

/// The neutral line the run view shows. Informational, not a failure: the
/// agent's work landed, only its wrapping was replaced.
pub fn fold_note(commits: &[AgentCommit]) -> String {
    if commits.is_empty() {
        return "Agent moved HEAD itself — its work was folded into Demeteo's commit".to_string();
    }
    let mut authors: Vec<&str> = Vec::new();
    for c in commits {
        if !authors.contains(&c.author.as_str()) {
            authors.push(&c.author);
        }
    }
    let noun = if commits.len() == 1 {
        "commit"
    } else {
        "commits"
    };
    let subjects = commits
        .iter()
        .map(|c| format!("\"{}\"", c.subject))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "Agent made {} {noun} as {} — folded into Demeteo's commit: {subjects}",
        commits.len(),
        authors.join(", "),
    )
}

#[cfg(test)]
#[path = "../../tests/domain/agent_commit_fold.rs"]
mod tests;
