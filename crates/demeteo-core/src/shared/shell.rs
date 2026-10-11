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

/// What a login shell's body prints before anything else, so the bytes ahead
/// of it can be told apart from the command's own.
///
/// A login shell sources the account's profile before it reads the body, and
/// whatever that profile writes to stdout arrives on the same pipe as the
/// command's answer with nothing between them. iTerm2's shell integration does
/// it on every start (three OSC 1337 sequences), and so does any rc that
/// prints a banner, a `fortune`, or a version manager's notice — on a remote
/// box as readily as on the desktop. A caller comparing or parsing stdout then
/// reads the account's dotfiles as the command's output: `echo hello` answered
/// `\e]1337;…\ahello`, and a project's own suite went red inside a sync gate
/// on a tree with nothing wrong in it.
///
/// Record separators rather than printable text, so the marker cannot be
/// produced by an rc that merely echoes a line.
pub const BODY_MARKER: &str = "\u{1e}demeteo:body\u{1e}";

/// The statement that emits [`BODY_MARKER`], for a body a login shell runs.
///
/// Empty for a non-login shell: `sh -c` sources nothing, so there is nothing
/// ahead of the body to separate it from — and its body stays the bare command
/// every existing caller of the default options was written against.
///
/// Applied by both adapters, like [`job_control_prefix`], and read back by
/// [`command_stdout`]. Spelled in octal because `printf`'s `\036` is POSIX and
/// `\x1e` is not.
pub fn body_marker_prefix(login_shell: bool) -> &'static str {
    if login_shell {
        "printf '\\036demeteo:body\\036'; "
    } else {
        ""
    }
}

/// A command's own stdout: everything after the first [`BODY_MARKER`].
///
/// The first, because the marker is the first thing the body does — anything
/// the command prints afterwards, an identical byte sequence included, is its
/// own output. A login shell's stdout with no marker in it is returned whole:
/// the body never started (the profile exited, the shell could not be
/// executed), and what was printed is the only account of why.
pub fn command_stdout(login_shell: bool, stdout: &[u8]) -> &[u8] {
    if !login_shell {
        return stdout;
    }
    let marker = BODY_MARKER.as_bytes();
    stdout
        .windows(marker.len())
        .position(|window| window == marker)
        .map_or(stdout, |at| &stdout[at + marker.len()..])
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
