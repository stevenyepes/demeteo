// Tests extracted from `crates/demeteo-core/src/shared/shell.rs` (mirrored-tests convention). `super` = that module.

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
