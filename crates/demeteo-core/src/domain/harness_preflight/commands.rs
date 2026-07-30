//! Which commands a project configured, and which binaries they will actually
//! try to execute.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::domain::models::WorktreeStrategy;

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
pub fn probeable_binaries(commands: &[&str]) -> Vec<String> {
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
    /// One entry of the `harnesses` map, named by
    /// [`ProbedCommand::harness`](super::report::ProbedCommand::harness).
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
pub fn labelled_commands(strategy: &WorktreeStrategy) -> Vec<(CommandSource, Option<&str>, &str)> {
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
pub fn configured_commands(strategy: &WorktreeStrategy) -> Vec<&str> {
    labelled_commands(strategy)
        .into_iter()
        .map(|(_, _, cmd)| cmd)
        .collect()
}

#[cfg(test)]
#[path = "../../../tests/domain/harness_preflight/commands.rs"]
mod tests;
