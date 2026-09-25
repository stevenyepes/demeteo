//! The git half of folding an agent's own commits back into Demeteo's; the
//! decision and the reason it exists are in
//! [`crate::domain::agent_commit_fold`].
//!
//! Must run after the turn and **before** the snapshot delta and
//! `commit_worktree_changes`: once HEAD is back at the pin, the agent's work
//! is uncommitted again, so the delta sees it and the pathspec exclusion
//! applies to it.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::adapters::worktree::git_ops::git_request;
use crate::domain::agent_commit_fold::{
    fold_note, parse_agent_commits, plan_fold, FoldOutcome, FoldPlan, HeadState,
    AGENT_COMMIT_LOG_FORMAT,
};
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::DomainEvent;

async fn git_line<const N: usize>(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    args: [&str; N],
) -> Option<String> {
    exec.run_program(machine_id, git_request(worktree_root, args))
        .await
        .ok()
        .map(|out| out.trim().to_string())
        .filter(|out| !out.is_empty())
}

/// Pin HEAD and the ref it points at. Called before every agent turn.
pub(crate) async fn capture_head(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
) -> HeadState {
    HeadState {
        sha: git_line(
            exec,
            machine_id,
            worktree_root,
            ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
        )
        .await,
        branch: git_line(
            exec,
            machine_id,
            worktree_root,
            ["symbolic-ref", "--quiet", "HEAD"],
        )
        .await,
    }
}

/// Undo whatever the agent did to HEAD since `pre`, keeping its working tree.
pub(crate) async fn fold_agent_commits(
    exec: &dyn ExecutionPort,
    machine_id: &str,
    worktree_root: &str,
    pre: &HeadState,
) -> FoldOutcome {
    let post = capture_head(exec, machine_id, worktree_root).await;
    let (branch, reset_to) = match plan_fold(pre, &post) {
        FoldPlan::Clean => return FoldOutcome::Clean,
        FoldPlan::CannotFold { reason } => return FoldOutcome::CannotFold { reason },
        FoldPlan::Fold { branch, reset_to } => (branch, reset_to),
    };

    let format = format!("--format={AGENT_COMMIT_LOG_FORMAT}");
    let range = format!("{reset_to}..HEAD");
    let commits = match exec
        .run_program(
            machine_id,
            git_request(worktree_root, ["log", &format, &range, "--"]),
        )
        .await
    {
        Ok(log) => parse_agent_commits(&log),
        Err(e) => {
            tracing::warn!(
                worktree = %worktree_root,
                error = %e,
                "fold_agent_commits: could not list the agent's commits; folding anyway",
            );
            Vec::new()
        }
    };

    if post.branch.as_deref() != Some(branch.as_str()) {
        if let Err(e) = exec
            .run_program(
                machine_id,
                git_request(worktree_root, ["symbolic-ref", "HEAD", &branch]),
            )
            .await
        {
            return FoldOutcome::CannotFold {
                reason: format!("could not point HEAD back at {branch}: {e}"),
            };
        }
    }
    // `--mixed`, never `--soft` — the module docs of
    // `domain::agent_commit_fold` say why.
    if let Err(e) = exec
        .run_program(
            machine_id,
            git_request(worktree_root, ["reset", "--mixed", "--quiet", &reset_to]),
        )
        .await
    {
        return FoldOutcome::CannotFold {
            reason: format!("could not reset {branch} to {reset_to}: {e}"),
        };
    }
    FoldOutcome::Folded { commits }
}

impl ExecutionDriver {
    /// Fold, then report. A fold that cannot happen leaves the worktree as
    /// the agent left it, so the commit that follows behaves as it always
    /// did; it is logged rather than failing a turn whose work is intact.
    pub(crate) async fn fold_agent_commits_into_turn(
        &self,
        step_id: &str,
        task_id: Option<&str>,
        machine_id: &str,
        worktree_root: &str,
        pre: &HeadState,
    ) {
        match fold_agent_commits(&*self.exec, machine_id, worktree_root, pre).await {
            FoldOutcome::Clean => {}
            FoldOutcome::CannotFold { reason } => {
                tracing::warn!(
                    feature_id = %self.f_id,
                    step_id = %step_id,
                    task_id = ?task_id,
                    reason = %reason,
                    "agent moved HEAD during its turn and it could not be folded",
                );
            }
            FoldOutcome::Folded { commits } => {
                let note = fold_note(&commits);
                tracing::info!(
                    feature_id = %self.f_id,
                    step_id = %step_id,
                    task_id = ?task_id,
                    "{note}",
                );
                let _ = self.notif.emit(&DomainEvent::AgentCommitsFolded {
                    feature_id: self.f_id.clone(),
                    step_id: step_id.to_string(),
                    task_id: task_id.map(str::to_string),
                    commits,
                    note,
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/step_executor/artifacts/agent_commits.rs"]
mod tests;
