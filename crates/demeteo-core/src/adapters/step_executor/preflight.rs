//! Bootstrap harness preflight (HB1) — resolve the project's configured
//! commands *before* the pipeline starts, so a misconfiguration costs seconds
//! at launch instead of an entire implement budget at `s-validate`.
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
//! # What "the configured commands" means (HB4)
//!
//! Every command the project configured, not just `test_command`:
//! `prepare_command`, `test_command`, and every value in the `harnesses` map. A
//! `prepare_command` naming a missing binary is as fatal as a `test_command`
//! doing so, and a step selecting a named harness runs *that* string — probing
//! only `test_command` would check something the run never executes. Probing is
//! `command -v` either way, so covering all of them costs nothing extra beyond
//! the probes themselves.
//!
//! # The bias, stated once
//!
//! **A false negative is cheap; a false positive blocks a legitimate launch.**
//! Everything below is built around that asymmetry: anything this module cannot
//! confidently resolve to a plain binary name is skipped rather than guessed at.
//! Missing a broken command means the user lands in today's behaviour. Wrongly
//! flagging a working one means they cannot start work at all.
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

use std::collections::BTreeSet;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::domain::models::WorktreeStrategy;
use crate::ports::execution::{ExecutionPort, ShellOptions};

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

/// What a configured command has to survive, stated once and rendered
/// everywhere it applies.
///
/// The engine already tells a user this when a baseline cannot be measured
/// (`baseline.rs`); the settings panel needs the identical sentence *before*
/// they have paid for a run. Both sites therefore read this constant instead of
/// each carrying its own wording — a second copy would drift, and the two would
/// then disagree about the two things nobody guesses: that the worktree a
/// harness runs in is a fresh `git worktree add` with no `node_modules` and no
/// `target/` (which is why `prepare_command` exists at all), and that a
/// watch-mode runner never exits, so it burns the entire wall-clock ceiling and
/// then fails.
pub const FRESH_CHECKOUT_REMEDIATION: &str =
    "Run the command below in a *fresh* checkout — that is what this step gets, with no \
     `node_modules` and no `target/`. If it needs an install step, set the project's \
     prepare command; if it hangs, it is most likely a watch-mode runner, which never \
     exits.";

/// Shell keywords, builtins and no-ops that either always resolve or are not
/// commands at all. Probing them yields nothing and risks a false positive on
/// a shell whose `command -v` disagrees with ours about builtins.
const SHELL_WORDS: &[&str] = &[
    ".", ":", "[", "alias", "bg", "break", "builtin", "case", "cd", "command", "continue",
    "declare", "do", "done", "echo", "elif", "else", "esac", "eval", "exec", "exit", "export",
    "false", "fi", "for", "function", "getopts", "hash", "if", "in", "jobs", "kill", "let",
    "local", "printf", "pwd", "read", "readonly", "return", "select", "set", "shift", "source",
    "test", "then", "time", "times", "trap", "true", "type", "ulimit", "umask", "unalias", "unset",
    "until", "wait", "while",
];

/// Characters that mean "this word is not a literal binary name": command or
/// parameter substitution, globs, redirections. A word containing any of them
/// is skipped — resolving it would require running the shell, which is the
/// thing we are trying to avoid doing at launch.
const UNRESOLVABLE: &[char] = &['$', '`', '(', ')', '<', '>', '*', '?', '{', '}', '"', '\''];

/// What the preflight established about a project's configured commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PreflightVerdict {
    /// The project configures **no command at all** — no `prepare_command`, no
    /// `test_command`, and no named harness. Nothing to probe — **not** a
    /// failure. A project may legitimately have no harness; the run proceeds
    /// and the validate step is told plainly that nothing ran (S12).
    ///
    /// It is deliberately *all three*: a project that configures only named
    /// harnesses has a harness, and reporting "none configured" at it would be
    /// false.
    NotConfigured,
    /// Every binary the configured commands name resolves on the login shell's
    /// `PATH`. Carries what was actually checked, so the phase detail can say
    /// so rather than claiming more than it verified.
    Resolved { probed: Vec<String> },
    /// At least one named binary does not resolve. The run is blocked: nothing
    /// downstream can turn "not installed" into a passing harness, and every
    /// step until `s-validate` would be paid for before anyone found out.
    MissingBinaries { missing: Vec<String> },
}

impl PreflightVerdict {
    /// Whether the launch may proceed.
    pub(crate) fn permits_launch(&self) -> bool {
        !matches!(self, PreflightVerdict::MissingBinaries { .. })
    }

    /// The `BootstrapProgress` status this verdict renders as.
    pub(crate) fn phase_status(&self) -> &'static str {
        match self {
            // Nothing to check is not the same as checked-and-fine, and the
            // stepper distinguishes them: `skipped` reads as "you have no
            // harness", which is information the user may want to act on.
            PreflightVerdict::NotConfigured => "skipped",
            PreflightVerdict::Resolved { .. } => "completed",
            PreflightVerdict::MissingBinaries { .. } => "failed",
        }
    }

    /// The human-facing `detail` line for the stepper, and — on the failing
    /// path — the error the feature terminates with. Names the binary and how
    /// to check it, because "not found" without a reproduce line sends people
    /// looking in the wrong shell.
    pub(crate) fn detail(&self) -> Option<String> {
        match self {
            PreflightVerdict::NotConfigured => Some(
                "This project configures no commands at all — no test command, no prepare \
                 command, and no named harnesses — so nothing will be run to verify the \
                 feature. Set at least a test command in project settings if you want the \
                 validate step to have evidence to judge."
                    .to_string(),
            ),
            PreflightVerdict::Resolved { probed } if probed.is_empty() => None,
            PreflightVerdict::Resolved { probed } => {
                Some(format!("Resolved on PATH: {}", probed.join(", ")))
            }
            PreflightVerdict::MissingBinaries { missing } => Some(format!(
                "The project's configured commands name {plural} the login shell cannot find: \
                 {list}. The run is stopped here because nothing downstream can make {them} \
                 appear — the validate step would fail on the same thing after the whole \
                 implementation had been paid for.\n\
                 Check with:\n\
                 \x20 bash -l -i -c 'command -v {first}'\n\
                 If that prints nothing, either export the tool's directory from ~/.profile or \
                 ~/.bashrc, or — if a version manager owns it (mise, asdf, nvm, pyenv, rbenv) — \
                 declare it in that manager's *global* config so every shell activates it. If \
                 the command itself is wrong, fix it in project settings.",
                plural = if missing.len() == 1 {
                    "a binary"
                } else {
                    "binaries"
                },
                list = missing.join(", "),
                them = if missing.len() == 1 { "it" } else { "them" },
                first = missing.first().map(String::as_str).unwrap_or(""),
            )),
        }
    }
}

/// Extract the binaries a set of user-authored command lines will actually try
/// to execute, skipping everything that cannot be resolved without running a
/// shell.
///
/// Splits each command on the operators that start a new command (`&&`, `||`,
/// `;`, `|`, newline), then takes each segment's command word — after stepping
/// over any leading `VAR=value` assignments, which are not commands. Words that
/// are shell builtins, or that contain substitution/glob/redirection
/// characters, are dropped: see the module's bias note.
///
/// Order-preserving and deduplicated **across the whole slice**, not merely
/// within one command: a command naming `cargo` three times probes it once, and
/// so does a project whose prepare, test and three harnesses all say `npm`.
/// That is what lets HB4 probe every configured command for the cost of the
/// distinct tools in them.
pub(crate) fn probeable_binaries(commands: &[&str]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();

    for segment in commands.iter().flat_map(|cmd| {
        cmd.split(['\n', ';'])
            .flat_map(|s| s.split("&&"))
            .flat_map(|s| s.split("||"))
            .flat_map(|s| s.split('|'))
    }) {
        let mut words = segment.split_whitespace();
        // Step over leading assignments: `RUST_LOG=debug cargo test` runs
        // `cargo`, not `RUST_LOG=debug`.
        let word = loop {
            match words.next() {
                Some(w) if w.contains('=') && !w.starts_with('=') => continue,
                Some(w) => break Some(w),
                None => break None,
            }
        };
        let Some(word) = word else { continue };

        if word.is_empty()
            || word.starts_with('-')
            || word.contains(UNRESOLVABLE)
            || SHELL_WORDS.contains(&word)
        {
            continue;
        }
        if seen.insert(word.to_string()) {
            out.push(word.to_string());
        }
    }
    out
}

/// Which project setting a probed command came from, so the settings panel can
/// put the answer back beside the field the user typed it into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandSource {
    /// `WorktreeStrategy::prepare_command`.
    Prepare,
    /// `WorktreeStrategy::test_command` — the resolution chain's tier 3.
    Test,
    /// One entry of the `harnesses` map, named by [`ProbedCommand::harness`].
    Harness,
}

/// Every command the project has actually configured, tagged with where it came
/// from, in probe order: `prepare_command`, then `test_command`, then each named
/// harness **sorted by name**.
///
/// The sort is not cosmetic: `harnesses` is a `HashMap`, so an unsorted walk
/// would vary the probe order — and therefore the order binaries appear in a
/// failure message — between two runs of the same project. Blank and
/// whitespace-only entries are dropped; a setting cleared to `""` is not a
/// configured command.
///
/// [`configured_commands`] is this list with the tags dropped, rather than a
/// second walk of the same three sources: the set the launch probes and the set
/// the settings panel displays have to be the same set, and deriving one from
/// the other is what makes that structural instead of a convention.
pub(crate) fn labelled_commands(
    strategy: &WorktreeStrategy,
) -> Vec<(CommandSource, Option<&str>, &str)> {
    fn live(cmd: &Option<String>) -> Option<&str> {
        cmd.as_deref().map(str::trim).filter(|c| !c.is_empty())
    }

    let mut out: Vec<(CommandSource, Option<&str>, &str)> = Vec::new();
    out.extend(live(&strategy.prepare_command).map(|c| (CommandSource::Prepare, None, c)));
    out.extend(live(&strategy.test_command).map(|c| (CommandSource::Test, None, c)));
    if let Some(harnesses) = &strategy.harnesses {
        let mut entries: Vec<(&String, &String)> = harnesses.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        out.extend(
            entries
                .into_iter()
                .map(|(name, cmd)| (CommandSource::Harness, Some(name.as_str()), cmd.trim()))
                .filter(|(_, _, cmd)| !cmd.is_empty()),
        );
    }
    out
}

/// Every configured command, in probe order. See [`labelled_commands`].
pub(crate) fn configured_commands(strategy: &WorktreeStrategy) -> Vec<&str> {
    labelled_commands(strategy)
        .into_iter()
        .map(|(_, _, cmd)| cmd)
        .collect()
}

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

// ── HB6: the same probe, attributed back to the settings that produced it ────

/// One binary a configured command names, and whether the machine could find
/// it.
///
/// A binary this module *skipped* (a builtin, a substitution, a glob) is absent
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
    /// a save is not gated on a probe (see the module header).
    pub blocks_launch: bool,
}

/// Attribute one [`PreflightVerdict`] back to the individual commands that
/// produced it.
///
/// Pure, and deliberately separate from the `async fn` that does the probing:
/// the whole mapping — which command owns which binary, and what the panel is
/// therefore entitled to claim — is decidable in a unit test with no port
/// double (AGENTS.md, "where a decision is allowed to live").
pub(crate) fn attribute_verdict(
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
