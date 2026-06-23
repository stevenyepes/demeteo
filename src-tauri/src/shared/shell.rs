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
