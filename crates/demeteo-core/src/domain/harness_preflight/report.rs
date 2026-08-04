//! HB6: one verdict, attributed back to the settings that produced it.

use serde::{Deserialize, Serialize};

use crate::domain::harness_preflight::commands::{
    labelled_commands, probeable_binaries, CommandSource,
};
use crate::domain::harness_preflight::verdict::{PreflightVerdict, FRESH_CHECKOUT_REMEDIATION};
use crate::domain::models::WorktreeStrategy;

/// One binary a configured command names, and whether the machine could find
/// it.
///
/// A binary the probe *skipped* (a builtin, a substitution, a glob) is absent
/// rather than present-and-unknown: the panel claims exactly what was checked,
/// no more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbedBinary {
    pub name: String,
    pub resolved: bool,
}

/// One configured command as the settings panel sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProbedCommand {
    pub source: CommandSource,
    /// The `harnesses` key, for [`CommandSource::Harness`]. `None` for the two
    /// project-wide settings, which have no name of their own.
    pub harness: Option<String>,
    pub command: String,
    pub binaries: Vec<ProbedBinary>,
}

/// The answer to "can this project's configured commands run on the machine
/// that will run them", per command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandProbeReport {
    /// **The machine that was actually asked.** Not decoration: on a project
    /// with a remote compute type the commands run there, not on the laptop
    /// showing the panel, and an indicator that does not say where it looked is
    /// a lie on exactly those projects.
    pub machine: String,
    /// Every configured command, in [`labelled_commands`] order.
    pub commands: Vec<ProbedCommand>,
    /// [`PreflightVerdict::detail`] verbatim — the same string a blocked launch
    /// terminates with, carrying the `bash -l -i -c` reproduce line. Rendered
    /// rather than paraphrased so the panel and the launch cannot drift apart.
    pub detail: Option<String>,
    /// [`FRESH_CHECKOUT_REMEDIATION`], for the same reason.
    pub guidance: String,
    /// Whether this verdict would stop a launch. Reported, never enforced here:
    /// a save is not gated on a probe (see
    /// `adapters::step_executor::preflight`'s module header).
    pub blocks_launch: bool,
}

/// Attribute one [`PreflightVerdict`] back to the individual commands that
/// produced it.
///
/// The whole mapping — which command owns which binary, and what the panel is
/// therefore entitled to claim — is decidable with no port double. That used to
/// be an aspiration stated in a doc comment beside the `async fn` that does the
/// probing; it is now a module boundary, so nothing here *can* reach a port
/// (AGENTS.md, "where a decision is allowed to live").
pub fn attribute_verdict(
    strategy: &WorktreeStrategy,
    machine: &str,
    verdict: &PreflightVerdict,
) -> CommandProbeReport {
    const NONE: &[String] = &[];
    let missing = match verdict {
        PreflightVerdict::MissingBinaries { missing } => missing.as_slice(),
        _ => NONE,
    };

    let commands = labelled_commands(strategy)
        .into_iter()
        .map(|(source, harness, command)| ProbedCommand {
            source,
            harness: harness.map(str::to_string),
            command: command.to_string(),
            binaries: probeable_binaries(&[command])
                .into_iter()
                .map(|name| ProbedBinary {
                    resolved: !missing.contains(&name),
                    name,
                })
                .collect(),
        })
        .collect();

    CommandProbeReport {
        machine: machine.to_string(),
        commands,
        detail: verdict.detail(),
        guidance: FRESH_CHECKOUT_REMEDIATION.to_string(),
        blocks_launch: !verdict.permits_launch(),
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/harness_preflight/report.rs"]
mod tests;
