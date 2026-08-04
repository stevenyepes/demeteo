//! Bootstrap harness preflight (HB1) — resolve the project's configured
//! commands *before* the pipeline starts, so a misconfiguration costs seconds
//! at launch instead of an entire implement budget at `s-validate`.
//!
//! What counts as a configured command, what a probe of them establishes, and
//! how that answer is attributed back to the settings panel are all pure, and
//! live in [`crate::domain::harness_preflight`]. What is left here is the
//! probing: two `async fn`s that run shell-specific probe variants through the
//! execution port, and the ceiling they run under.
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

use crate::domain::models::{ScriptVariants, WorktreeStrategy};
use crate::ports::execution::{ExecutionPort, ScriptRequest};

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

    let script_bodies: Vec<&str> = commands
        .into_iter()
        .flat_map(|script| [script.posix.as_deref(), script.powershell.as_deref()])
        .flatten()
        .collect();
    let binaries = probeable_binaries(&script_bodies);
    if binaries.is_empty() {
        // A command made entirely of builtins and substitutions. Nothing to
        // assert, and asserting nothing is the honest outcome — not a failure.
        return PreflightVerdict::Resolved { probed: vec![] };
    }

    let mut missing = Vec::new();
    let mut probed = Vec::new();
    for bin in binaries {
        // The port selects the shell variant at the execution boundary. That
        // keeps a local Windows run, local POSIX run, and remote run on the
        // same behavioural contract rather than teaching callers about hosts.
        match exec
            .run_script(
                machine_str,
                ScriptRequest {
                    variants: binary_probe_script(&bin),
                    cwd: Some(cwd.to_string()).filter(|path| !path.is_empty()),
                    timeout: Some(timeout),
                    ..ScriptRequest::default()
                },
            )
            .await
        {
            Ok(out) if !out.trim().is_empty() => probed.push(bin),
            Ok(_) => missing.push(bin),
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

/// Confirm the two tools every local bootstrap needs before it performs any
/// git operation. `run_script` is deliberate: on Windows, selecting the
/// PowerShell variant proves that PowerShell 7 is available before the feature
/// branch or configured scripts can start.
pub(crate) async fn validate_bootstrap_tools(
    exec: &dyn ExecutionPort,
    machine_str: &str,
    cwd: &str,
    timeout: Duration,
) -> Result<(), String> {
    exec.run_script(
        machine_str,
        ScriptRequest {
            variants: ScriptVariants {
                posix: Some("git --version".to_string()),
                powershell: Some("git --version".to_string()),
            },
            cwd: Some(cwd.to_string()).filter(|path| !path.is_empty()),
            timeout: Some(timeout),
            ..ScriptRequest::default()
        },
    )
    .await
    .map(|_| ())
    .map_err(|error| {
        if error.starts_with("configuration error:") {
            error
        } else {
            format!(
                "configuration error: Git is required to start a feature. Install Git and ensure it is on PATH ({error})"
            )
        }
    })
}

fn binary_probe_script(binary: &str) -> ScriptVariants {
    let posix = format!("command -v {}", crate::paths::shell_escape_posix(binary));
    let powershell_name = binary.replace('\'', "''");
    let powershell = format!(
        "$command = Get-Command -Name '{powershell_name}' -CommandType Application -ErrorAction SilentlyContinue; if ($null -eq $command) {{ exit 1 }}; $command.Source"
    );
    ScriptVariants {
        posix: Some(posix),
        powershell: Some(powershell),
    }
}

/// Whether an `ExecutionPort` error from a binary probe means "the name did
/// not resolve" rather than "the probe itself could not run".
///
/// Only a genuine non-zero exit counts. Transport and timeout failures carry
/// their own prefixes (D3) and must not be read as a missing binary — that
/// would block a launch over a dropped connection.
fn is_not_found(err: &str) -> bool {
    !err.starts_with(crate::ports::execution::TRANSPORT_ERROR_PREFIX)
        && !err.starts_with(crate::ports::execution::TIMEOUT_ERROR_PREFIX)
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
