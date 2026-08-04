//! Bootstrap harness preflight (HB1) — resolve the project's configured
//! commands *before* the pipeline starts, so a misconfiguration costs seconds
//! at launch instead of an entire implement budget at `s-validate`.
//!
//! What counts as a configured command, what a probe of them establishes, and
//! how that answer is attributed back to the settings panel are all pure, and
//! live in [`crate::domain::harness_preflight`]. What is left here is the
//! probing: two `async fn`s that spawn `command -v` under an interactive login
//! shell, and the ceiling they run under.
//!
//! # Why this exists at all
//!
//! The orchestrator executes exactly one user-authored command per run, and it
//! executes it late: `run_harness_first` is gated on a step declaring a
//! `verifier`, which in the standard pipeline is `s-validate` alone. Everything
//! before that point trusts the configuration blindly. So `cargo: not found`
//! surfaces after research, tickets, spec and the whole implement sequence have
//! been paid for — and surfaces wearing the costume of a failed feature.
//!
//! # Why a bootstrap phase and not a node
//!
//! It has to hold for runs of *any* workflow, including one a user drew
//! themselves with no baseline node in it. A graph node can only protect the
//! graphs that contain it. This runs in `run_bootstrap_tail_inner`, before any
//! graph is walked, so nothing can opt out of it.
//!
//! # Why only probes
//!
//! It resolves binaries; it does not *run* anything the project configured —
//! not the suite, and not `prepare_command`. Running belongs to the
//! `baseline-harness` command node (P4.2a), which sits at the head of the graph
//! — the same point in the timeline for a fraction of the wall-clock here,
//! where every launch pays it. A phase that adds a minute to "Launch" before
//! anything visible happens would be paid by every user on every run to catch a
//! problem the next node catches anyway.
//!
//! # The same probe, at configuration time (HB6)
//!
//! [`probe_project_commands`] is this module's second entry point: the settings
//! panel asks the *same* question about the *same* commands on the *same*
//! machine, and gets the answer attributed back to the individual settings the
//! user is looking at. It shares [`PreflightVerdict::detail`] verbatim rather
//! than paraphrasing it, so what the panel says and what a blocked launch says
//! can never disagree.
//!
//! It is an indicator there, not a gate. Nothing about a settings-time answer
//! may block a save: a user may legitimately configure a command for a machine
//! that is not the one they are sitting at (the remote runner especially), and
//! the gate belongs at launch, where which machine will run it is known.

use std::time::Duration;

use crate::adapters::local::execution::NO_POSIX_SHELL_ERROR;
use crate::domain::models::WorktreeStrategy;
use crate::ports::execution::{ExecutionPort, ShellOptions};

// Re-exported rather than re-homed: `commands/project.rs` and
// `application/projects.rs` name these by full path through this module, and
// the frontend reads the JSON they serialise to.
pub use crate::domain::harness_preflight::commands::{
    configured_commands, labelled_commands, probeable_binaries, CommandSource,
};
pub use crate::domain::harness_preflight::report::{
    attribute_verdict, CommandProbeReport, ProbedBinary, ProbedCommand,
};
pub use crate::domain::harness_preflight::verdict::{PreflightVerdict, FRESH_CHECKOUT_REMEDIATION};

/// Per-probe ceiling for the harness preflight, in seconds.
///
/// Its own constant rather than the run's `wall_cap_s`, because the two answer
/// different questions: `wall_cap_s` is "how long may a build take" (30 min by
/// default), while this bounds a single `command -v` on an already-connected
/// machine. Anything beyond a few seconds means the machine or the shell is
/// unwell, not that the binary is slow to find — and an expiry is treated as
/// *no evidence* rather than a missing binary, so erring short is safe.
///
/// Shared by the launch phase and the settings panel (HB6) on purpose: two
/// ceilings would let the panel report a binary the launch cannot find, or the
/// reverse, and the whole value of probing at configuration time is that it is
/// the same answer arriving earlier.
pub const PREFLIGHT_PROBE_TIMEOUT_S: u64 = 20;

/// Probe every binary the project's configured commands name, on the machine
/// that will run them.
///
/// Takes the whole `WorktreeStrategy` rather than one command because the three
/// sources travel together and are read as a set: `prepare_command`,
/// `test_command`, and the `harnesses` map. Their union is deduplicated, so the
/// cost is one probe per distinct tool, not per command.
///
/// A free function over the one port it needs, rather than a method on
/// `ExecutionDriver` — the driver carries twenty-odd ports this never reads,
/// and a decision reachable only through it is a decision no test can see
/// (AGENTS.md, "where a decision is allowed to live").
///
/// `command -v` runs under an **interactive login shell** for the same reason
/// the harness itself does: the binaries live on the *user's* `PATH`, which
/// only a login profile establishes, and `mise`/`asdf`/`nvm` shims only
/// activate in an interactive one. Probing under a bare `sh -c` would report
/// half a developer's toolchain missing.
/// An empty `cwd` means "the adapter's default working directory" rather than
/// an empty path. `command -v` needs the login shell, not the repo, and the
/// settings-time caller (HB6) has no checkout to point at — the project may not
/// be provisioned on that machine yet. Naming a directory that does not exist
/// would fail every probe at spawn time and read as a missing toolchain, which
/// is exactly the false positive this module is built around avoiding.
pub(crate) async fn probe_configured_commands(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    cwd: &str,
    strategy: &WorktreeStrategy,
    timeout: Duration,
) -> PreflightVerdict {
    let commands = configured_commands(strategy);
    if commands.is_empty() {
        return PreflightVerdict::NotConfigured;
    }

    let binaries = probeable_binaries(&commands);
    if binaries.is_empty() {
        // A command made entirely of builtins and substitutions. Nothing to
        // assert, and asserting nothing is the honest outcome — not a failure.
        return PreflightVerdict::Resolved { probed: vec![] };
    }

    let opts = ShellOptions {
        cwd: Some(cwd.to_string()).filter(|c| !c.is_empty()),
        timeout: Some(timeout),
        ..ShellOptions::login_interactive()
    };

    let mut missing = Vec::new();
    let mut probed = Vec::new();
    for bin in binaries {
        // `command -v` exits non-zero when the name does not resolve, which the
        // port surfaces as `Err`. A transport failure or a timeout also lands
        // in `Err` and is indistinguishable here — so it is deliberately NOT
        // treated as "missing": blocking a launch because the network hiccuped
        // is the false positive this module exists to avoid. Those cases fall
        // through as resolved, and the run behaves exactly as it does today.
        //
        // The one exception is a machine with no shell to ask with, which is a
        // permanent fact about the machine rather than a blip: see
        // [`is_missing_posix_shell`].
        match exec
            .run_command_with(
                machine_str,
                &format!("command -v {}", crate::paths::shell_escape_posix(&bin)),
                opts.clone(),
            )
            .await
        {
            Ok(out) if !out.trim().is_empty() => probed.push(bin),
            Ok(_) => missing.push(bin),
            Err(e) if is_missing_posix_shell(&e) => return PreflightVerdict::MissingPosixShell,
            Err(e) if is_not_found(&e) => missing.push(bin),
            Err(_) => probed.push(bin),
        }
    }

    if missing.is_empty() {
        PreflightVerdict::Resolved { probed }
    } else {
        PreflightVerdict::MissingBinaries { missing }
    }
}

/// Whether an `ExecutionPort` error from `command -v` means "the name did not
/// resolve" rather than "the probe itself could not run".
///
/// Only a genuine non-zero exit counts. Transport and timeout failures carry
/// their own prefixes (D3) and must not be read as a missing binary — that
/// would block a launch over a dropped connection.
fn is_not_found(err: &str) -> bool {
    !err.starts_with(crate::ports::execution::TRANSPORT_ERROR_PREFIX)
        && !err.starts_with(crate::ports::execution::TIMEOUT_ERROR_PREFIX)
}

/// Whether the probe failed because the machine has no POSIX shell to run it
/// with — a statement about the machine, not about the binary being asked for.
///
/// It is a transport-class error (the probe never ran), so the rule above would
/// otherwise read it as no evidence and let every binary through as resolved.
/// That is right for a dropped connection and wrong here: the next probe fails
/// identically, the harness fails identically, and the launch would proceed to
/// spend an entire implement budget before `s-validate` discovered it. Answered
/// once, from the first probe, because a second asks the same question of the
/// same absent shell.
///
/// The one Windows behaviour visible from this module, and it is visible only
/// as a string every transport could in principle raise —
/// [`NO_POSIX_SHELL_ERROR`] is the vocabulary, not a `#[cfg]`.
fn is_missing_posix_shell(err: &str) -> bool {
    err.strip_prefix(crate::ports::execution::TRANSPORT_ERROR_PREFIX)
        .is_some_and(|rest| rest.starts_with(NO_POSIX_SHELL_ERROR))
}

/// Probe the project's configured commands **at configuration time** and report
/// the answer per command (HB6).
///
/// The same probe the launch runs, on the same machine, against the commands
/// currently in the panel — including ones the user has typed but not yet
/// saved, which is the entire point: the most valuable place to say a
/// `test_command` is wrong is where it was just authored.
///
/// Runs with the adapter's default working directory: see
/// [`probe_configured_commands`].
pub async fn probe_project_commands(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    strategy: &WorktreeStrategy,
    timeout: Duration,
) -> CommandProbeReport {
    let verdict = probe_configured_commands(exec, machine_str, "", strategy, timeout).await;
    attribute_verdict(strategy, machine_str, &verdict)
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/preflight_tests.rs"]
mod preflight_tests;
