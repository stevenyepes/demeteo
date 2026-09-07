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

/// The case the hardcoded `bash` got wrong: a zsh account's tool-manager
/// shims are declared in `~/.zshrc`, so only zsh's own `-l -i` puts them on
/// PATH.
#[test]
fn a_posix_login_shell_is_taken_as_it_stands() {
    assert_eq!(
        posix_login_shell(Some("/usr/bin/zsh")),
        Some("/usr/bin/zsh")
    );
    assert_eq!(posix_login_shell(Some("/bin/bash")), Some("/bin/bash"));
    assert_eq!(posix_login_shell(Some("dash")), Some("dash"));
}

/// A shell that cannot parse `set +m; export K='V'; …` is refused, so the
/// body still runs — under bash, whose rc a tool manager writes when it is
/// the one configured.
#[test]
fn a_shell_that_cannot_read_the_body_falls_back() {
    assert_eq!(posix_login_shell(Some("/usr/bin/fish")), None);
    assert_eq!(posix_login_shell(Some("/bin/csh")), None);
    assert_eq!(posix_login_shell(Some("/usr/bin/nu")), None);
}

#[test]
fn an_unset_or_blank_shell_is_no_answer() {
    assert_eq!(posix_login_shell(None), None);
    assert_eq!(posix_login_shell(Some("")), None);
    assert_eq!(posix_login_shell(Some("   ")), None);
}
