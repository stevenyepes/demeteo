//! Single source of truth for POSIX shell escaping. The previous
//! codebase had three copies that drifted apart (`paths::shell_escape_posix`,
//! `adapters/merge::shell_escape`, `commands/feature_lifecycle::shell_escape`).
//!
//! The escape rules implemented here:
//! - Keep the legacy `paths::shell_escape_posix` semantics (the "safe chars" fast path)
//!   and home directory shortcut preservation (`~`, `~/`).
//! - Wrap in single quotes only when unsafe characters are present.
//! - Replace every `'` inside with `'\''` (close quote, escaped literal
//!   quote, open quote again).

/// Build the `export K='V'; ` prefix string for a set of environment
/// variables, ready to prepend to a shell command body. Values are
/// single-quote-escaped with the standard `'\''` trick so arbitrary
/// content is safe.
///
/// This is the **single** construction both `LocalSubprocessAdapter` and
/// `SshClientAdapter` use for `run_command_with`, so the two transports
/// export the caller's environment identically (decision D2). Ordering is
/// the map's natural key order — pass a `BTreeMap` for determinism.
///
/// The exports are emitted *inside* the shell body (after a login shell has
/// sourced its profile), so the caller's values win over anything the
/// profile sets — matching how `spawn_interactive` composes the agent env.
pub fn export_prefix<'a, I>(env: I) -> String
where
    I: IntoIterator<Item = (&'a String, &'a String)>,
{
    let mut out = String::new();
    for (k, v) in env {
        let escaped = v.replace('\'', "'\\''");
        out.push_str(&format!("export {}='{}'; ", k, escaped));
    }
    out
}

/// Assemble the shell *body* — the string that becomes the argument to
/// `bash -l -c` / `sh -c` — from an optional cwd, an env-export prefix, and
/// the caller's command. When `cwd` is `Some`, a `cd <cwd> &&` is prepended
/// so a failed `cd` aborts before the command runs (rather than silently
/// executing in the wrong directory). Shared by both adapters so the body is
/// byte-identical across transports for the same inputs.
pub fn command_body(cwd: Option<&str>, exports: &str, cmd: &str) -> String {
    match cwd {
        Some(dir) => format!("cd {} && {}{}", escape_posix(dir), exports, cmd),
        None => format!("{}{}", exports, cmd),
    }
}

/// Prefix that turns **job control off** for an interactive shell.
///
/// `bash -i` enables monitor mode, which puts every background job in a
/// *process group of its own*. That quietly defeats killing a command's tree:
/// `ShellOptions::timeout` signals the child's process group, and with monitor
/// mode on a `sleep 60 &` sits in a different group and survives — the exact
/// orphaned-process case the deadline exists to prevent.
///
/// We only ever pass `-i` so the user's `~/.bashrc` is sourced, because that is
/// where `mise`/`asdf`/`nvm` put their PATH activation (see
/// `ShellOptions::interactive`). A batch `-c` invocation has no use for job
/// control, so switching it back off costs nothing and keeps the whole command
/// in one signalable group.
///
/// Applied by both adapters so the body stays byte-identical across transports
/// for the same options (D2).
pub fn job_control_prefix(interactive: bool) -> &'static str {
    if interactive {
        "set +m; "
    } else {
        ""
    }
}

/// The account's own login shell, when it is one that reads the bodies built
/// above; `None` for anything else, which means "use `bash`".
///
/// Hardcoding bash was the older answer, on the grounds that `$SHELL` may name
/// a shell that never sources `~/.bashrc`. That is true, and it is the failure
/// rather than the reason: `mise`/`asdf`/`nvm` write their PATH activation
/// into the rc of the shell the account actually logs into, so a zsh account's
/// shims are declared in `~/.zshrc`, which bash never reads. Launched from a
/// desktop entry — where the session PATH carries no shims either — Demeteo
/// then resolves none of the tool-managed harnesses and reports each one as
/// absent, `pi` and a `mise`-installed `opencode` alike.
///
/// The family check is what keeps the body parseable: it carries `set +m` and
/// `export K='V'`, which fish and csh reject outright. Those fall back to
/// bash, which is also what a tool manager configures when it is set up under
/// them.
pub fn posix_login_shell(shell_var: Option<&str>) -> Option<&str> {
    let path = shell_var.map(str::trim).filter(|p| !p.is_empty())?;
    let name = path.rsplit('/').next()?;
    matches!(
        name,
        "bash" | "zsh" | "ksh" | "ksh93" | "mksh" | "dash" | "ash" | "sh"
    )
    .then_some(path)
}

/// Escape `s` so it is safe to interpolate into a POSIX shell command
/// line as a single argument.
pub fn escape_posix(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s == "~" {
        return "~".into();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        return format!("~/{}", escape_posix(rest));
    }
    if s.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '=' | ':' | ',' | '@')
    }) {
        return s.to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

#[cfg(test)]
#[path = "../../tests/shared/shell.rs"]
mod tests;
