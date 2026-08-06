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
//! ## The `match platform` below is not the branch AGENTS.md §2 forbids
//!
//! It keys on the platform of the machine the worktree is on, not on how the
//! work reached it, and the platform arrives through `resolve_platform` rather
//! than a `cfg!`. `docs/EXECUTION_PARITY.md` carries that argument in full,
//! including the test that separates the two — do not delete this match on
//! parity grounds without reading it.
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
/// [`PlatformPlacement::prefix`] carries are the same bytes. Two renderings that
/// differed by a separator would be two blocks to keep in step.
fn platform_context_section(platform: Platform) -> String {
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
pub(crate) fn place_platform_context(
    platform: Option<Platform>,
    template: &str,
) -> PlatformPlacement {
    let section = platform.map(platform_context_section).unwrap_or_default();
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
