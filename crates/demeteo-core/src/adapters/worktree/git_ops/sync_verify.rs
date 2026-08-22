//! Whether the tree a merge produced earned its way to origin.
//!
//! Both halves of a sync ask it. The clean half asks after committing the
//! merge and withholds the push; the conflicted half asks in
//! `adapters::step_executor::sync_resolve`, before there is anything to
//! withhold, and refuses to commit. One runner, two phrasings; that it is one
//! runner is
//! [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage)'s
//! doing, and its reasons are there.
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
    match run_merge_gate(exec, machine, gate, opts, None).await {
        GateVerdict::Clear | GateVerdict::Stopped => None,
        GateVerdict::Failed { command, error } => {
            verify_failure_stage(&error)?;
            Some(format!(
                "The merge is committed on this branch and the project's checks failed in \
                 it, so it was not pushed.\n\n$ {}\n{}",
                command, error
            ))
        }
    }
}

/// What running a project's gate came back with, **before** anyone has decided
/// whether it is a verdict about the tree.
///
/// Raw rather than a refusal because the refusal sentence has to name what is
/// being withheld, and the two callers are withholding different things:
/// sharing one sentence would make the other a lie. Each applies
/// [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage)
/// to [`Failed::error`](Self::Failed) and writes its own.
#[derive(Debug, PartialEq)]
pub(crate) enum GateVerdict {
    /// Nothing ran, or it ran and passed: no harness configured, the prepare
    /// command could not run, or the harness exited zero.
    Clear,
    /// The harness ran and came back `Err` — a red build, a dropped transport,
    /// or an expired deadline, undistinguished.
    Failed { command: String, error: String },
    /// A stop arrived while the harness was in flight.
    Stopped,
}

/// Run `gate` in the worktree `opts` names, and report what happened.
///
/// `cancel` is the caller's Stop, and `None` is "nothing can stop this" rather
/// than "not stopped": the run is raced against the watch by
/// [`run_harness_command`](crate::adapters::step_executor::harness_shell::run_harness_command),
/// where dropping the run future is what kills the process group. A gate that
/// could not be stopped once it started was a build the user could only escape
/// by restarting the app.
pub(crate) async fn run_merge_gate(
    exec: &dyn ExecutionPort,
    machine: &str,
    gate: MergeGate<'_>,
    opts: ShellOptions,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> GateVerdict {
    use crate::adapters::step_executor::harness_shell::run_harness_command;

    let Some(harness) = gate.harness else {
        return GateVerdict::Clear;
    };
    let cancel = cancel.unwrap_or_else(|| tokio::sync::watch::channel(false).1);

    if let Some(prepare) = gate.prepare {
        match run_harness_command(exec, cancel.clone(), machine, prepare, opts.clone()).await {
            None => return GateVerdict::Stopped,
            Some(Err(err)) => {
                tracing::warn!(
                    machine = %machine,
                    worktree = opts.cwd.as_deref().unwrap_or_default(),
                    error = %err,
                    "sync gate skipped: the project's prepare command failed, so a red \
                     harness here would not be about the merge",
                );
                return GateVerdict::Clear;
            }
            Some(Ok(_)) => {}
        }
    }

    match run_harness_command(exec, cancel, machine, harness, opts).await {
        None => GateVerdict::Stopped,
        Some(Ok(_)) => GateVerdict::Clear,
        Some(Err(error)) => GateVerdict::Failed {
            command: harness.to_string(),
            error,
        },
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/sync_verify.rs"]
mod tests;
