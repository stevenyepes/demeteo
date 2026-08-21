//! Whether the tree a clean merge produced earned its way to origin.
//!
//! `git merge` reconciles *text*. Two edits that never touch the same line
//! merge without complaint, and the result can still be a tree that does not
//! build: a field added to a struct on the base branch against a new literal
//! of that struct added on the feature branch live in different files, so
//! there is nothing for git to conflict on and nothing for the conflict
//! resolver to open — and every check on the pull request goes red on a merge
//! Demeteo reported as clean. That is the gap this stage exists to close, and
//! it is the reason the gate is a *build*, not a smarter merge: no textual
//! strategy can see it.
//!
//! A free function over the *one* port it needs rather than a method on
//! `GitOpsHelper`, and it takes the resolved
//! [`ShellOptions`](crate::ports::execution::ShellOptions) rather than the
//! settings repository they are derived from — which is what makes it
//! reachable from a test with a single scripted double (AGENTS.md §3). The
//! policy itself is
//! [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage);
//! this module only runs the commands and hands it their errors.

use crate::domain::sync_failure::verify_failure_stage;
use crate::ports::execution::{ExecutionPort, ShellOptions};
use crate::ports::worktree_ops::MergeGate;

/// Run the gate in `wt_path` and return the reason the push must be withheld,
/// or `None` when it may proceed.
///
/// `None` covers three different situations on purpose, because they are the
/// same instruction: the project named no harness, the harness passed, or
/// nobody was in a position to say it did not. Only the last is a judgement,
/// and it is
/// [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage)'s
/// — a harness the transport cut short or the deadline abandoned never ran, and
/// a build that never ran is not a red build. Withholding on one of those would
/// strand a merge that is already committed on the branch and tell the user
/// their tree is broken on the strength of nothing, which is strictly worse
/// than the unverified push this whole module exists to replace.
///
/// A failing **prepare** is the same category and not a red build either. It
/// says the worktree could not be brought to a state where the question is
/// answerable — a registry that would not resolve, a codegen step that needs a
/// tool this machine lacks — and answering "your merge is broken" to that is
/// the environment-versus-regression mistake `domain::harness_failure` was
/// written to stop the verifier making.
pub(crate) async fn merge_gate_refusal(
    exec: &dyn ExecutionPort,
    machine: &str,
    gate: MergeGate<'_>,
    opts: ShellOptions,
) -> Option<String> {
    let harness = gate.harness?;

    if let Some(prepare) = gate.prepare {
        if let Err(err) = exec.run_command_with(machine, prepare, opts.clone()).await {
            tracing::warn!(
                machine = %machine,
                worktree = opts.cwd.as_deref().unwrap_or_default(),
                error = %err,
                "sync gate skipped: the project's prepare command failed, so a red \
                 harness here would not be about the merge",
            );
            return None;
        }
    }

    match exec.run_command_with(machine, harness, opts).await {
        Ok(_) => None,
        Err(err) => {
            verify_failure_stage(&err)?;
            Some(format!(
                "The merge is committed on this branch and the project's checks failed in \
                 it, so it was not pushed.\n\n$ {}\n{}",
                harness, err
            ))
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/sync_verify.rs"]
mod tests;
