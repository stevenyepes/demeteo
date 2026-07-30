//! The shell one user-authored command runs under, and the race that stops it.
//!
//! Two callers run shell somebody typed into project settings — the harness-
//! first pass and the `command` node (P3.5) — and every question either of them
//! has about *how* is answered here: which shell, which cwd, which deadline,
//! and what Stop does to a command already in flight. Sharing the answer is the
//! point: two call sites that disagree about the shell disagree about whether a
//! project's `npm test` resolves at all, and two that disagree about the race
//! disagree about whether Stop works.
//!
//! Free functions over the one port each needs, not methods on
//! `ExecutionDriver`: the options read `app_settings` and the race reads `exec`
//! plus the cancel channel, which is three of the driver's eighteen ports and
//! the reason this is reachable from a test at all (AGENTS.md §3).
//!
//! The login-interactive shell is **unconditional**, and this is the file where
//! a `cfg` or a machine-flag lookup would be tempting. It is deliberate; see
//! [`harness_shell_options`] for the detached-run failure that a conditional
//! produced.

use crate::ports::db::AppSettingsRepository;
use crate::ports::execution::{ExecutionPort, ShellOptions};
use tokio::sync::watch;

/// Build the [`ShellOptions`](crate::ports::execution::ShellOptions) the
/// prepare/test harness runs under: the worktree as an explicit cwd (D2 —
/// never rely on ambient state) under an **interactive login shell**,
/// unconditionally.
///
/// A prepare/test command is user-authored shell (`cargo test`, `npm test`,
/// `pytest`) whose binaries live on the *user's* `PATH`, which only a login
/// shell's profile establishes — and only an *interactive* one activates
/// `mise`/`asdf`/`nvm` shims, which hide behind the standard `~/.bashrc`
/// non-interactive guard. So the harness always needs the same shell the
/// agent probe already hardcodes (`ShellOptions::login_interactive`).
///
/// This deliberately does **not** consult the machine's `use_login_shell`
/// flag. That flag is only reachable through the SSH adapter — i.e. an
/// *attached* run, where the desktop app drives commands over the wire. A
/// **detached** run executes inside `demeteo-runner` on the target box
/// itself, which registers its project as `compute_type: "local"`; `"local"`
/// is a sentinel that short-circuits the DB lookup and yields a synthetic
/// machine whose `use_login_shell` is hardcoded `None` (see
/// `machine_resolver::local_machine`). Gating on the flag therefore forced
/// every detached harness through a bare non-login `sh -c` no matter what
/// the user had ticked in the UI, and a bare `cargo` in the harness command
/// died with "cargo: not found" — while the *implement* step sailed through,
/// because the agent binary is resolved to an absolute path up front and so
/// never needed `PATH` at all.
///
/// `pub(crate)` because the `command` node type (P3.5) runs
/// user-authored shell for the same reason under the same
/// constraints — sharing the decision beats re-deriving it there.
///
/// # Deadline
///
/// The options carry the run's `wall_cap_s` as an explicit
/// [`timeout`](crate::ports::execution::ShellOptions::timeout). Without one
/// the harness was the only unbounded wait in a step: `wall_cap_s` itself is
/// enforced inside `stream_agent_turn`, and the harness runs *before* any
/// turn starts, so a command that never exits hung the step until the app
/// restarted. It reuses the existing user-configurable cap rather than
/// introducing a second knob — a harness is bounded by the same "how long
/// may one step take" answer an agent turn is.
///
/// The `command` node overrides this with its own `spec.timeout`, which is
/// why this is a default rather than a floor — and so does each resolved
/// harness, which carries the same ceiling as *its own*
/// [`deadline_s`](crate::domain::verifier::ResolvedHarness::deadline_s).
/// The cap answers "how long may one command take", so N gates get N
/// ceilings rather than a slice each; the sum a step may spend is therefore
/// unbounded in the number of gates its author declared. See that field for
/// why dividing would be worse.
pub(crate) fn harness_shell_options(
    app_settings: &dyn AppSettingsRepository,
    wt_path: &str,
) -> ShellOptions {
    ShellOptions {
        cwd: Some(wt_path.to_string()),
        timeout: Some(std::time::Duration::from_secs(harness_ceiling_s(
            app_settings,
        ))),
        ..ShellOptions::login_interactive()
    }
}

/// The wall-clock ceiling one prepare/harness command may consume, in
/// seconds. Read through the same resolver every agent-turn call site uses,
/// so one preferences change moves both.
pub(crate) fn harness_ceiling_s(app_settings: &dyn AppSettingsRepository) -> u64 {
    crate::application::timeouts::resolve_effective(app_settings).wall_cap_s
}

/// Run one prepare/harness command, racing it against cancellation.
///
/// Dropping the run future is what actually stops the work — the local
/// adapter kills the command's process group on drop — so the `biased`
/// select is the mechanism, not just a status check. Mirrors what
/// `steps/command.rs` already does for the `command` node type: both are
/// user-authored shell built from [`harness_shell_options`], and they must
/// not disagree about whether Stop works.
pub(crate) async fn run_harness_command(
    exec: &dyn ExecutionPort,
    mut cancel_watch: watch::Receiver<bool>,
    machine_str: &str,
    cmd: &str,
    opts: ShellOptions,
) -> Option<Result<String, String>> {
    let cancelled = async move {
        // `wait_for` also resolves — as `Err` — when the sender is dropped.
        // That is "nobody can cancel this any more", not "this was
        // cancelled", so park forever and let the command decide the
        // outcome rather than killing a healthy step during teardown.
        if cancel_watch.wait_for(|c| *c).await.is_err() {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => None,
        r = exec.run_command_with(machine_str, cmd, opts) => Some(r),
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/harness_shell.rs"]
mod tests;
