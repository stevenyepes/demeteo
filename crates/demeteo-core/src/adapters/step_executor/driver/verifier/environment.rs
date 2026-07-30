//! Raising the terminal environment-not-ready signal.
//!
//! Three shapes of "no amount of editing the code can fix this" — a binary the
//! shell could not find, a script the runner could not find, and everything C6
//! or the baseline classified as an environment fault — and the one channel all
//! of them reach the user through.
//!
//! Free functions over [`EnvironmentSignal`] rather than methods on
//! `ExecutionDriver`: the whole stage reads three of its eighteen ports and
//! touches none of the other fifteen, which is what makes it assertable against
//! three narrow doubles (AGENTS.md §3). What the *messages* say is
//! [`harness_remediation`](crate::domain::harness_remediation); what counts as
//! never having run is
//! [`harness_failure`](crate::domain::harness_failure). Both are pure. This is
//! the choreography between them.

use crate::adapters::step_executor::driver::ExecutionDriver;
use crate::domain::harness_failure::{
    build_missing_task_message, detect_missing_command, detect_missing_task,
};
use crate::domain::harness_outcome::HarnessRun;
use crate::domain::harness_remediation::build_environment_message;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::ports::db::{FeatureRepository, NotificationRepository};
use crate::ports::notification::{DomainEvent, NotificationPort};

/// The three collaborators that carry "this machine cannot run your project"
/// to the user, plus the feature they are carrying it about.
///
/// One struct rather than four parameters because they never travel apart: the
/// notification row and the live event are two halves of one signal — HB9 wants
/// news that survives a refresh *and* a toast — and neither is addressable
/// without the feature id. It is also the seam every test of this stage is
/// built on, since all three are `dyn`.
pub(crate) struct EnvironmentSignal<'a> {
    pub features: &'a dyn FeatureRepository,
    pub notifications: &'a dyn NotificationRepository,
    pub notif: &'a dyn NotificationPort,
    pub feature_id: &'a FeatureId,
}

impl ExecutionDriver {
    /// The three ports this stage reads, and nothing else the driver holds.
    pub(crate) fn environment_signal(&self) -> EnvironmentSignal<'_> {
        EnvironmentSignal {
            features: self.features.as_ref(),
            notifications: self.notifications.as_ref(),
            notif: self.notif.as_ref(),
            feature_id: &self.f_id,
        }
    }
}

/// The exit-127 fast path: the shell could not find a binary a harness
/// command itself invokes. That is objectively an environment gap — the
/// code never ran, so no amount of editing it can help. Escalate straight
/// to the terminal `Environment` error rather than spending a `Verdict`
/// retry (which re-runs the agent against a gate that cannot pass) plus a
/// triage agent turn to reach the same conclusion on the *next* attempt.
/// This skips `should_triage`'s reproduce-unchanged requirement on purpose:
/// a 127 is deterministic, not flaky.
///
/// A named function rather than inline code because HB2c gave it a second
/// caller:
/// `run_harness_first` asks it about the **unsubtracted** failure set,
/// before the baseline is consulted, so a missing binary stays terminal
/// even when it was equally missing at the base. `None` means no failure
/// here names a binary the shell could not find.
fn missing_command_error(
    sig: &EnvironmentSignal<'_>,
    step_exec: &StepExecution,
    machine_str: &str,
    wt_path: &str,
    failures: &[HarnessRun],
) -> Option<crate::domain::verifier::VerifierError> {
    let (failure, missing) = failures
        .iter()
        .find_map(|f| detect_missing_command(&f.cmd, &f.output).map(|m| (f, m)))?;
    let cmd = failure.cmd.as_str();
    let msg = build_environment_message(
        machine_str,
        wt_path,
        cmd,
        &format!(
            "The shell could not find `{}` on PATH (exit 127), so the command never ran.",
            missing
        ),
        &format!(
            "Make `{missing}` *discoverable* on this machine — installed is not enough, it \
             has to be on the PATH of a fresh interactive login shell, which is what the \
             harness runs commands under. Check it with:\n\
             \x20 bash -l -i -c 'command -v {missing}'\n\
             If that prints nothing, either export the tool's directory from ~/.profile or \
             ~/.bashrc, or — if a version manager owns it (mise, asdf, nvm, pyenv, rbenv) — \
             declare it in that manager's *global* config so every shell activates it, not \
             just the directories that ask for it.",
        ),
    );
    notify_environment_not_ready(sig, step_exec, &msg);
    tracing::warn!(
        feature_id = %sig.feature_id,
        step_id = %step_exec.step_id.0,
        cmd = %cmd,
        missing = %missing,
        "harness command not found on PATH — terminating without retries"
    );
    Some(crate::domain::verifier::VerifierError::Environment(msg))
}

/// The sibling of [`missing_command_error`] for
/// the failures that *do* reach an exit code: a task runner that ran, but
/// was asked for a script or target this worktree does not define.
///
/// Same conclusion, same fail-safe direction, different remediation — see
/// [`build_missing_task_message`] for why reusing the 127 path's wording
/// would send the user after a package that was never missing.
fn missing_task_error(
    sig: &EnvironmentSignal<'_>,
    step_exec: &StepExecution,
    machine_str: &str,
    wt_path: &str,
    failures: &[HarnessRun],
) -> Option<crate::domain::verifier::VerifierError> {
    let (failure, missing) = failures
        .iter()
        .find_map(|f| detect_missing_task(&f.cmd, &f.output).map(|m| (f, m)))?;
    let cmd = failure.cmd.as_str();
    let msg = build_missing_task_message(machine_str, wt_path, cmd, &missing);
    notify_environment_not_ready(sig, step_exec, &msg);
    tracing::warn!(
        feature_id = %sig.feature_id,
        step_id = %step_exec.step_id.0,
        cmd = %cmd,
        runner = %missing.runner,
        missing = %missing.name,
        "harness command named a {} this worktree does not define — terminating without retries",
        missing.noun()
    );
    Some(crate::domain::verifier::VerifierError::Environment(msg))
}

/// "The command never ran" — both shapes of it, in the order that gives the
/// most specific remediation.
///
/// A binary the shell could not find (exit 127) and a script/target the
/// runner could not find (exit 1) are one category: the code was never
/// exercised, so a `Verdict` would redirect an agent to fix something that
/// was never tested, and it would reproduce identically on every retry until
/// the budget ran out. Both therefore skip `should_triage`'s
/// reproduce-unchanged requirement and terminate directly — neither is flaky.
///
/// The 127 check goes first because it is the stronger claim: if the binary
/// itself is absent, "your project's script list is wrong" would be a
/// misdiagnosis of a machine that cannot run the tool at all.
pub(crate) fn command_never_ran_error(
    sig: &EnvironmentSignal<'_>,
    step_exec: &StepExecution,
    machine_str: &str,
    wt_path: &str,
    failures: &[HarnessRun],
) -> Option<crate::domain::verifier::VerifierError> {
    missing_command_error(sig, step_exec, machine_str, wt_path, failures)
        .or_else(|| missing_task_error(sig, step_exec, machine_str, wt_path, failures))
}

/// Persist + emit the terminal environment-not-ready signal (C6.3), fired
/// *immediately* on triage (no wasted retries first). Mirrors the
/// `RetryBudgetExhausted` persistence path so the bell shows it after a
/// refresh, plus a live event for the toast.
///
/// `pub(crate)` because the baseline node terminates on the same signal
/// (HB9): a gate that cannot run is the same news to the user whether the
/// engine noticed it at the head of the graph or at validate, and it must
/// arrive through the same channel — one that survives a refresh — rather
/// than as a step error string only the node panel shows.
pub(crate) fn notify_environment_not_ready(
    sig: &EnvironmentSignal<'_>,
    step_exec: &StepExecution,
    message: &str,
) {
    if let Ok(Some(feature)) = sig.features.get(sig.feature_id) {
        let notification = crate::domain::models::Notification {
            id: format!("notif-{}", crate::paths::now_ms()),
            project_id: feature.project_id.0.clone(),
            feature_id: sig.feature_id.0.clone(),
            kind: crate::domain::models::NotificationKind::EnvironmentNotReady,
            message: message.to_string(),
            feature_url: Some(format!(
                "/projects/{}/features/{}",
                feature.project_id.0, sig.feature_id.0
            )),
            read: false,
            created_at: crate::paths::now_ms(),
        };
        let _ = sig.notifications.add(notification);
    }
    let _ = sig.notif.emit(&DomainEvent::EnvironmentNotReady {
        feature_id: sig.feature_id.clone(),
        step_id: step_exec.step_id.0.clone(),
        reason: message.to_string(),
    });
}

#[cfg(test)]
#[path = "../../../../../tests/infrastructure/step_executor/verifier/environment.rs"]
mod tests;
