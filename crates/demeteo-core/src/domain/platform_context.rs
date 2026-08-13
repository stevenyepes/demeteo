//! What OS the agent is standing on — the one thing the prompt never said.
//!
//! Every other environment signal an agent step emits is POSIX-flavoured: the
//! `{{harness_baseline}}` block quotes the project's gate commands verbatim on
//! every platform, worktree paths appear as bare strings, and the model's own
//! prior for "a coding agent in a git worktree" is Linux. On a Windows desktop
//! that adds up to an agent that reaches for POSIX-shaped *tooling decisions*
//! it was never told it had — the observed failure being a harness reaching for
//! bash utilities and then translating the project's own gate commands into
//! something else. These blocks are the correction, and they are the whole of
//! it.
//!
//! ## Why there is more than one Windows block
//!
//! `docs/WINDOWS_PARITY.md` splits execution into two planes by *authorship*:
//! bodies the user wrote run as POSIX under Git for Windows' bash, and
//! everything Demeteo spawns is argv with no shell. An agent holding a command
//! tool is a third author, and which interpreter sits behind that tool is the
//! harness's choice, not Demeteo's — so the one thing a block may promise about
//! command syntax varies per agent while everything else in it does not.
//!
//! ## Neither dimension of the `match` is a branch AGENTS.md forbids
//!
//! The platform keys on the machine the worktree is on, not on how the work
//! reached it, and arrives through `resolve_platform` rather than a `cfg!`.
//! `docs/EXECUTION_PARITY.md` carries that argument in full, including the test
//! that separates the two — do not delete this match on parity grounds without
//! reading it.
//!
//! The shell is a declared
//! [`AgentCapabilities`](crate::ports::agent_runtime::AgentCapabilities) field
//! and not the agent kind, which is the mechanism §3 asks for rather than the
//! `match` on kind it rules out. Adding a sixth agent therefore does not touch
//! this file: it declares what its command tool runs and the right block
//! follows. Keep it that way — an
//! [`AgentKind`](crate::domain::models::AgentKind) reaching this module is the
//! smell, and the honest answer for a harness nobody has run on Windows is
//! [`WindowsAgentShell::Unknown`], not the nearest-looking arm.
//!
//! ## Why a POSIX target gets nothing
//!
//! The block exists to correct a default, and on Linux and macOS the default is
//! already right — every command string in the prompt is a POSIX body, the
//! paths are `/`-rooted, and the model's prior matches. A paragraph saying so
//! would spend tokens on every turn of every step to restate what has never
//! been observed to go wrong, and would change the rendered prompt for the two
//! platforms carrying all of today's traffic with no defect behind the change.
//!
//! The door is left open by matching on [`Platform`] rather than asking
//! [`Platform::is_posix`]: a macOS-specific correction (BSD `sed -i` against
//! GNU `sed -i` is the standing candidate) would be a new arm here, not a new
//! mechanism.

use crate::domain::models::{Platform, WindowsAgentShell};

/// The `{{platform_context}}` token a template may use to place the block
/// itself.
const TOKEN: &str = "{{platform_context}}";

const HEADING: &str = "## Platform — Windows";

/// What the agent may assume about the syntax of a command it writes itself.
///
/// The only bullet a shell declaration changes, which is why the rest are
/// assembled around it rather than copied per block.
fn command_tool_bullet(shell: WindowsAgentShell) -> &'static str {
    match shell {
        WindowsAgentShell::GitBash => {
            "- Shell commands here are POSIX script bodies, executed by the bash that Git for
  Windows installs — Demeteo resolves its absolute path and invokes it for you, so
  you do not have to find it. `sh`/`bash` syntax, pipes, `&&`, `$VAR`, and the Unix
  utilities Git for Windows bundles all work: a POSIX command IS runnable on this
  machine."
        }
        WindowsAgentShell::PowerShell => {
            "- Commands you author through your own command tool run in PowerShell. Use
  PowerShell cmdlets or native programs. Do not use bare POSIX-only utilities
  such as `printf`, `sed`, or `find`, and do not invoke bare `bash`; Windows
  ships PowerShell 5.1, which parses neither `&&` nor a POSIX `$VAR` expansion."
        }
        WindowsAgentShell::Unknown => {
            "- Which interpreter runs a command you author here has not been established for
  your harness, so assume nothing about shell syntax. Prefer a script the project
  already ships over a command you compose, and verify a POSIX-only utility such
  as `printf`, `sed`, or `find` works before depending on it."
        }
    }
}

/// The instruction the whole block exists to carry, plus the one thing that
/// makes it followable.
///
/// The prohibition is worthless on its own to an agent whose own shell cannot
/// run the body it is forbidden to rewrite — it has been told what not to do
/// and given no way to comply, and five shipped workflow templates ask it to
/// run `{{test_command}}` by hand. The wrapper closes that: the body inside the
/// quotes is unchanged, which is the property `run-topology-conformance.sh`
/// asserts and the property a rewrite destroys.
fn harness_lane_bullet(shell: WindowsAgentShell, bash: Option<&str>) -> String {
    let prohibition = "- You MUST NOT translate any command quoted elsewhere in this prompt into
  PowerShell or `cmd.exe`. The project's prepare, test, gate and harness commands
  are POSIX bodies, and Demeteo runs them through the bash Git for Windows
  installs, exactly as written; a rewritten command is a different command, and
  this work will be judged by running the original.";

    if shell == WindowsAgentShell::GitBash {
        return prohibition.to_string();
    }

    let interpreter = match bash {
        Some(path) => format!("'{path}'"),
        None => "'<the bash.exe inside your Git for Windows installation>'".to_string(),
    };
    format!(
        "{prohibition}
  To run one yourself, hand it to that bash unchanged rather than rewriting it:

      & {interpreter} -lc '<the command exactly as quoted above>'

  Wrapping a command is not translating it — the text inside the single quotes
  must stay byte-for-byte what this prompt quoted."
    )
}

const PATHS_BULLET: &str = r#"- Paths you are given — the worktree, artifacts, attachments — are Windows-shaped:
  drive-lettered and `\`-separated. `\` is this machine's path separator. Hand them
  to Windows programs as written, quote any path containing spaces, and quote them
  inside a bash body too, where `\` escapes."#;

fn windows_block(shell: WindowsAgentShell, bash: Option<&str>) -> String {
    format!(
        "{HEADING}\n\nThe worktree you are working in is on **Windows**. It is not Linux and not macOS.\n\n{}\n{}\n{PATHS_BULLET}\n\n---\n\n",
        command_tool_bullet(shell),
        harness_lane_bullet(shell, bash),
    )
}

/// The platform block for `platform` and `shell`, or `""` when this combination
/// needs no correction.
///
/// Self-contained: it carries its own trailing `---` rule and blank line, so the
/// text a template places through `{{platform_context}}` and the text
/// [`PlatformPlacement::prefix`] carries are the same bytes. Two renderings that
/// differed by a separator would be two blocks to keep in step.
fn platform_context_section(
    platform: Platform,
    shell: WindowsAgentShell,
    bash: Option<&str>,
) -> String {
    match platform {
        Platform::Windows => windows_block(shell, bash),
        Platform::Linux | Platform::MacOS => String::new(),
    }
}

/// True when the template places the block itself.
///
/// The only thing this gates is *duplication*. The block is otherwise
/// unconditional, and that is load-bearing: prompt templates are user-authored
/// and live in the DB, so no template on any existing install can reference a
/// token introduced today. A block that only rendered when asked for by name
/// would reach precisely the zero installs that need it — which is the same
/// reasoning that put a safety net behind `{{retry_feedback_section}}`
/// (`append_retry_feedback_section`, in the agent step's prompt builder).
fn template_uses_platform_context(template: &str) -> bool {
    template.contains(TOKEN)
}

/// Where one render puts the block: what `{{platform_context}}` binds to, and
/// what still has to go in front of the rendered prompt.
///
/// The two are alternatives and at most one is ever non-empty, which is why
/// they arrive together. Asked separately — a section here, a "does the
/// template name the token" there — the choice between them is spelled in the
/// prompt builder, and there are three of those: the agent step, the sequence
/// task, and the sequence planner. Each is then a place the block can go
/// missing — and the one that matters most is the sequence task, since that is
/// the step that writes the code and runs the gate command — or be emitted
/// twice, ~1.3 KB on every turn of every Windows step, with nothing in
/// `domain/` able to see either.
pub(crate) struct PlatformPlacement {
    /// What `{{platform_context}}` renders as.
    pub(crate) bound: String,
    /// What goes in front of the rendered prompt — empty when the template
    /// asked for the block by name.
    pub(crate) prefix: String,
}

/// Decide that placement.
///
/// The prefix goes at the front, alongside the Operating Boundary, because the
/// block reframes command text that appears *later* in the prompt — the harness
/// briefing's gate commands most of all. A correction the model reads after the
/// thing it corrects is a correction it has already had the chance to act
/// against.
///
/// `platform` is `None` when the execution port declined to name the machine's
/// OS. Nothing is placed then rather than a guess: an agent told the wrong OS is
/// worse off than one told nothing, which is the state every prompt was in
/// before the block existed.
///
/// `shell` is what the agent's runtime declares its command tool runs; callers
/// read it through
/// [`windows_agent_shell_for`](crate::adapters::agent::registry::AgentRegistry::windows_agent_shell_for),
/// which answers [`Unknown`](WindowsAgentShell::Unknown) for a kind it does not
/// recognise.
///
/// `bash` is the absolute interpreter path to quote, from
/// [`quotable_bash_path`](crate::shared::win::quotable_bash_path). `None`
/// degrades the wrapper to a description of where to find it rather than
/// inventing a literal: Git for Windows installs to at least five different
/// roots (`shared/win/posix_shell.rs` enumerates them), so a guess is wrong more
/// often than not.
pub(crate) fn place_platform_context(
    platform: Option<Platform>,
    shell: WindowsAgentShell,
    bash: Option<&str>,
    template: &str,
) -> PlatformPlacement {
    let section = platform
        .map(|platform| platform_context_section(platform, shell, bash))
        .unwrap_or_default();
    if template_uses_platform_context(template) {
        PlatformPlacement {
            bound: section,
            prefix: String::new(),
        }
    } else {
        PlatformPlacement {
            bound: String::new(),
            prefix: section,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/domain/platform_context.rs"]
mod tests;
