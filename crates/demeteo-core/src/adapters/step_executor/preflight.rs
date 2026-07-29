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

use std::collections::BTreeSet;
use std::time::Duration;

use crate::domain::models::WorktreeStrategy;
use crate::ports::execution::{ExecutionPort, ShellOptions};

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

/// Every command the project has actually configured, in probe order:
/// `prepare_command`, then `test_command`, then each named harness **sorted by
/// name**.
///
/// The sort is not cosmetic: `harnesses` is a `HashMap`, so an unsorted walk
/// would vary the probe order — and therefore the order binaries appear in a
/// failure message — between two runs of the same project. Blank and
/// whitespace-only entries are dropped; a setting cleared to `""` is not a
/// configured command.
pub(crate) fn configured_commands(strategy: &WorktreeStrategy) -> Vec<&str> {
    fn live(cmd: &Option<String>) -> Option<&str> {
        cmd.as_deref().map(str::trim).filter(|c| !c.is_empty())
    }

    let mut out: Vec<&str> = Vec::new();
    out.extend(live(&strategy.prepare_command));
    out.extend(live(&strategy.test_command));
    if let Some(harnesses) = &strategy.harnesses {
        let mut entries: Vec<(&String, &String)> = harnesses.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        out.extend(
            entries
                .into_iter()
                .map(|(_, cmd)| cmd.trim())
                .filter(|cmd| !cmd.is_empty()),
        );
    }
    out
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
        cwd: Some(cwd.to_string()),
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

#[cfg(test)]
#[path = "../../../tests/infrastructure/step_executor/preflight_tests.rs"]
mod preflight_tests;
