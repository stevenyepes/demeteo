//! Whether the tree a merge produced earned its way to origin.
//!
//! Both halves of a sync run this one gate — the clean merge and, in
//! `adapters::step_executor::sync_resolve`, a resolved conflict — for
//! [`verify_failure_stage`](crate::domain::sync_failure::verify_failure_stage)'s
//! reasons.
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
/// [`Failed`](GateVerdict::Failed) is the only verdict that can withhold, and
/// only where `verify_failure_stage` reads it as one about the tree — every
/// other answer lets the push through, for that function's reasons.
pub(crate) async fn merge_gate_refusal(
    exec: &dyn ExecutionPort,
    machine: &str,
    gate: MergeGate<'_>,
    opts: ShellOptions,
) -> Option<String> {
    match run_merge_gate(exec, machine, gate, opts, None).await {
        GateVerdict::NotGated
        | GateVerdict::Passed
        | GateVerdict::Unprepared { .. }
        | GateVerdict::Stopped => None,
        GateVerdict::Failed { error } => {
            verify_failure_stage(&error)?;
            Some(format!(
                "The merge is committed on this branch and the project's checks failed in \
                 it, so it was not pushed.\n\n$ {}\n{}",
                gate.harness.unwrap_or_default(),
                error
            ))
        }
    }
}

/// What running a project's gate came back with, **before** anyone has decided
/// what to do about it.
#[derive(Debug, PartialEq)]
pub(crate) enum GateVerdict {
    /// The project named no harness, so there was never a question.
    NotGated,
    /// The harness ran in a prepared tree and exited zero.
    Passed,
    /// The harness ran and came back `Err` — a red build, a dropped transport,
    /// or an expired deadline, undistinguished. Which of those it is, is
    /// [`verify_failure_stage`]'s to say.
    Failed { error: String },
    /// The tree could not be brought to a state where the question is
    /// answerable: the project's prepare command failed here. A registry that
    /// would not resolve or a codegen step this machine lacks a tool for says
    /// nothing about the merge, and answering "your merge is broken" to it is
    /// the environment-versus-regression mistake
    /// [`harness_failure`](crate::domain::harness_failure) was written to stop
    /// the verifier making.
    Unprepared { error: String },
    /// A stop arrived while prepare or the harness was in flight.
    Stopped,
}

/// What `gate.prepare` left behind.
#[derive(Debug, PartialEq)]
pub(crate) enum GatePrepare {
    /// The project named no harness, or none of its own preparation: there is
    /// nothing here to bring a tree to.
    NotNeeded,
    Ready,
    Failed(String),
    Stopped,
}

/// Bring the worktree to a state where the harness's answer is about the merge.
pub(crate) async fn run_gate_prepare(
    exec: &dyn ExecutionPort,
    machine: &str,
    gate: MergeGate<'_>,
    opts: ShellOptions,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> GatePrepare {
    use crate::adapters::step_executor::harness_shell::run_harness_command;

    let (Some(_), Some(prepare)) = (gate.harness, gate.prepare) else {
        return GatePrepare::NotNeeded;
    };
    let cancel = cancel.unwrap_or_else(|| tokio::sync::watch::channel(false).1);

    match run_harness_command(exec, cancel, machine, prepare, opts.clone()).await {
        None => GatePrepare::Stopped,
        Some(Ok(_)) => GatePrepare::Ready,
        Some(Err(err)) => {
            tracing::warn!(
                machine = %machine,
                worktree = opts.cwd.as_deref().unwrap_or_default(),
                stage = "prepare",
                error = %err,
                "sync gate reached no verdict: the project's prepare command failed, so a \
                 red harness here would not be about the merge",
            );
            GatePrepare::Failed(err)
        }
    }
}

/// Run the project's own checks against whatever the worktree now holds.
///
/// Takes the command rather than the gate, so there is no `prepare` field
/// within reach to run a second time.
pub(crate) async fn run_gate_harness(
    exec: &dyn ExecutionPort,
    machine: &str,
    harness: Option<&str>,
    opts: ShellOptions,
    cancel: Option<tokio::sync::watch::Receiver<bool>>,
) -> GateVerdict {
    use crate::adapters::step_executor::harness_shell::run_harness_command;

    let Some(harness) = harness else {
        return GateVerdict::NotGated;
    };
    let cancel = cancel.unwrap_or_else(|| tokio::sync::watch::channel(false).1);

    match run_harness_command(exec, cancel, machine, harness, opts).await {
        None => GateVerdict::Stopped,
        Some(Ok(_)) => GateVerdict::Passed,
        Some(Err(error)) => GateVerdict::Failed { error },
    }
}

/// Prepare and then the harness, which is the whole of the gate for a caller
/// holding a merge it has already committed.
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
    match run_gate_prepare(exec, machine, gate, opts.clone(), cancel.clone()).await {
        GatePrepare::Stopped => GateVerdict::Stopped,
        GatePrepare::Failed(error) => GateVerdict::Unprepared { error },
        GatePrepare::NotNeeded | GatePrepare::Ready => {
            run_gate_harness(exec, machine, gate.harness, opts, cancel).await
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/infrastructure/worktree/git_ops/sync_verify.rs"]
mod tests;
