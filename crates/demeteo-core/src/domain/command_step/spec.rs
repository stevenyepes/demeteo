//! What a `command` node was told to run.

use std::time::Duration;

use crate::domain::models::StepConfig;

/// The validated `command` payload, parsed once per dispatch out of the
/// v1 [`StepConfig`] fields (v2 storage is P3.6's prerequisite; the
/// migration copies these verbatim into the node's `config`).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CommandSpec {
    /// The authored shell command. Empty **only** when
    /// [`measure_baseline`](Self::measure_baseline) is set, which is the one
    /// mode where the commands to run come from the project rather than the
    /// workflow.
    pub command: String,
    /// Worktree-relative, already validated as non-escaping. `None` =
    /// worktree root.
    pub cwd: Option<String>,
    pub env_allowlist: Vec<String>,
    pub timeout: Option<Duration>,
    /// Unset reads as `false`: see [`StepConfig::idempotent`].
    pub idempotent: bool,
    /// Measure the harness baseline rather than run `command`
    /// (`docs/HARNESS_BASELINE.md` HB2b). See
    /// [`StepConfig::measure_baseline`].
    pub measure_baseline: bool,
}

/// Parse + validate a step's command config. `Err` is an author-facing
/// message; every case is a definition bug that no retry can fix, so the
/// caller returns
/// [`StepOutcome::NonRetryable`](crate::adapters::step_executor::steps::StepOutcome::NonRetryable)
/// and
/// [`CommandNodeHandler::lint`](crate::adapters::step_executor::steps::command::CommandNodeHandler::lint)
/// surfaces the same problems in the builder *before* a run is ever started.
pub(crate) fn parse_spec(step: &StepConfig) -> Result<CommandSpec, String> {
    let measure_baseline = step.measure_baseline.unwrap_or(false);
    // A baseline node's commands come from the *project* — its
    // `prepare_command` and the harnesses that gate validation — so demanding
    // one in the workflow would be asking the author for a string they cannot
    // know. Every other command node still owes one: it is the entire step.
    let command = match step
        .command
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        Some(cmd) => cmd.to_string(),
        None if measure_baseline => String::new(),
        None => return Err("command node declares no `command` to run".to_string()),
    };

    let cwd = match step.cwd.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        None => None,
        Some(dir) => {
            validate_relative_cwd(dir)?;
            Some(dir.trim_matches('/').to_string())
        }
    };

    let timeout = match step.timeout_secs {
        None => None,
        Some(0) => return Err("`timeout_secs` must be greater than zero".to_string()),
        Some(secs) => Some(Duration::from_secs(secs)),
    };

    Ok(CommandSpec {
        command,
        cwd,
        env_allowlist: step.env_allowlist.clone(),
        timeout,
        idempotent: step.idempotent.unwrap_or(false),
        measure_baseline,
    })
}

/// A command's cwd may not leave the worktree it was handed. Rooted
/// paths, `~`, and any `..` segment are refused: the node runs in a
/// disposable worktree precisely so its blast radius is bounded, and a
/// cwd that climbs out of it silently un-bounds that.
///
/// The verdict is the same on every host OS. A workflow authored on one
/// desktop is linted on another and executed by an always-Linux runner, so
/// a cwd Windows would resolve outside the worktree has to be refused
/// wherever it is read — which rules out [`std::path::Path::is_absolute`],
/// whose answer is `cfg`-dependent: on Unix it reads `C:\Windows` as a
/// relative directory named `C:`.
pub(crate) fn validate_relative_cwd(dir: &str) -> Result<(), String> {
    if dir.starts_with('~') || is_rooted(dir) {
        return Err(format!("`cwd` must be worktree-relative, got '{dir}'"));
    }
    if dir.split(is_separator).any(|seg| seg == "..") {
        return Err(format!("`cwd` must not escape the worktree, got '{dir}'"));
    }
    Ok(())
}

fn is_separator(c: char) -> bool {
    c == '/' || c == '\\'
}

/// Every spelling of "this path does not start at the directory it was
/// handed": POSIX absolute, both UNC forms, the Windows current-drive root
/// `\x`, and a drive prefix — including bare `C:foo`, which Windows resolves
/// against that drive's own working directory rather than this one.
fn is_rooted(dir: &str) -> bool {
    let mut chars = dir.chars();
    match (chars.next(), chars.next()) {
        (Some(first), _) if is_separator(first) => true,
        (Some(drive), Some(':')) => drive.is_ascii_alphabetic(),
        _ => false,
    }
}

#[cfg(test)]
#[path = "../../../tests/domain/command_step/spec.rs"]
mod tests;
