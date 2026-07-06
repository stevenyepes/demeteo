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
mod tests {
    use super::*;

    #[test]
    fn empty_string_returns_quoted_empty() {
        assert_eq!(escape_posix(""), "''");
    }

    #[test]
    fn plain_string_fast_path() {
        assert_eq!(escape_posix("hello"), "hello");
    }

    #[test]
    fn single_quote_is_escaped() {
        assert_eq!(escape_posix("it's"), "'it'\\''s'");
    }

    #[test]
    fn path_with_spaces_quoted() {
        assert_eq!(
            escape_posix("/usr/local/bin space"),
            "'/usr/local/bin space'"
        );
    }

    #[test]
    fn path_without_spaces_fast_path() {
        assert_eq!(escape_posix("/usr/local/bin"), "/usr/local/bin");
    }

    #[test]
    fn shell_metacharacters_neutralized() {
        let escaped = escape_posix("a;b&c$d");
        assert_eq!(escaped, "'a;b&c$d'");
    }

    #[test]
    fn unicode_passes_through_but_quoted() {
        let escaped = escape_posix("/home/用户/repo");
        assert_eq!(escaped, "'/home/用户/repo'");
    }

    #[test]
    fn quote_around_quote() {
        let escaped = escape_posix("a'b'c");
        assert_eq!(escaped, "'a'\\''b'\\''c'");
    }

    #[test]
    fn tilde_expansion_preserved() {
        assert_eq!(escape_posix("~"), "~");
        assert_eq!(escape_posix("~/foo bar"), "~/'foo bar'");
        assert_eq!(escape_posix("~/foo/bar"), "~/foo/bar");
    }
}
