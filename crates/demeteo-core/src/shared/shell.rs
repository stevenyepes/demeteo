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
