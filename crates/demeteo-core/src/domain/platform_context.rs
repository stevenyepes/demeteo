//! What OS the agent is standing on — the one thing the prompt never said.
//!
//! Every other environment signal an agent step emits is POSIX-flavoured: the
//! `{{harness_baseline}}` block quotes the project's gate commands verbatim on
//! every platform, worktree paths appear as bare strings, and the model's own
//! prior for "a coding agent in a git worktree" is Linux. On a Windows desktop
//! that adds up to an agent that reaches for POSIX-shaped *tooling decisions*
//! it was never told it had — the observed failure being a harness reaching for
//! bash utilities and then translating the project's own gate commands into
//! something else. This block is the correction, and it is the whole of it.
//!
//! ## Why this is not a transport branch
//!
//! A reviewer meeting a `match platform` in a prompt builder will read it as
//! the thing AGENTS.md §2 forbids — branching on transport in calling code. It
//! is not, and the distinction is exact: this keys on the **platform of the
//! machine the worktree is on**, which is orthogonal to how the work got there.
//! A Linux box driven by the local subprocess adapter and the same box driven
//! over SSH render byte-identical prompts; a Windows desktop and a Linux desktop
//! render different ones through the *same* transport. The `ExecutionPort`
//! contract is untouched — the platform arrives through `resolve_platform`,
//! which every transport answers by observing its own target rather than by
//! naming itself.
//!
//! The parity guarantee is that a feature behaves identically regardless of
//! which transport ran it (`docs/EXECUTION_PARITY.md`). Telling an agent it is
//! on Windows when it is on Windows is required *by* that guarantee: the
//! alternative is one prompt, describing a POSIX machine, rendered onto a host
//! that has no `/usr/bin` — which is a transport-invisible divergence in agent
//! behaviour, i.e. the exact failure the guarantee exists to exclude.
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

use crate::domain::models::Platform;

/// The `{{platform_context}}` token a template may use to place the block
/// itself.
const TOKEN: &str = "{{platform_context}}";

const WINDOWS_BLOCK: &str = r#"## Platform — Windows

The worktree you are working in is on **Windows**. It is not Linux and not macOS.

- Shell commands here are POSIX script bodies, executed by the bash that Git for
  Windows installs — Demeteo resolves its absolute path and invokes it for you, so
  you do not have to find it. `sh`/`bash` syntax, pipes, `&&`, `$VAR`, and the Unix
  utilities Git for Windows bundles all work: a POSIX command IS runnable on this
  machine.
- You MUST NOT translate any command quoted elsewhere in this prompt into
  PowerShell or `cmd.exe`. The project's gate and harness commands are POSIX bodies
  run under that bash exactly as written; a rewritten command is a different
  command, and this work will be judged by running the original.
- Paths you are given — the worktree, artifacts, attachments — are Windows-shaped:
  drive-lettered and `\`-separated. `\` is this machine's path separator. Hand them
  to Windows programs as written, and quote them inside a bash body, where `\`
  escapes.

---

"#;

/// The platform block for `platform`, or `""` when this platform needs no
/// correction.
///
/// Self-contained: it carries its own trailing `---` rule and blank line, so the
/// text a template places through `{{platform_context}}` and the text
/// [`inject_platform_context`] prepends are the same bytes. Two renderings that
/// differed by a separator would be two blocks to keep in step.
pub(crate) fn platform_context_section(platform: Platform) -> String {
    match platform {
        Platform::Windows => WINDOWS_BLOCK.to_string(),
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
pub(crate) fn template_uses_platform_context(template: &str) -> bool {
    template.contains(TOKEN)
}

/// Prepend the platform block, or return `prompt` unchanged when the platform
/// needs none.
///
/// At the front, alongside the Operating Boundary, because it reframes command
/// text that appears *later* in the prompt — the harness briefing's gate
/// commands most of all. A correction the model reads after the thing it
/// corrects is a correction it has already had the chance to act against.
pub(crate) fn inject_platform_context(prompt: &str, platform: Platform) -> String {
    let section = platform_context_section(platform);
    if section.is_empty() {
        prompt.to_string()
    } else {
        format!("{}{}", section, prompt)
    }
}

#[cfg(test)]
#[path = "../../tests/domain/platform_context.rs"]
mod tests;
