// Tests extracted from `src-tauri/src/terminal.rs` (mirrored-tests convention). `super` = that module.

use super::branch_bootstrap_line;

/// `None` (no pipeline context, e.g. `ProjectHome`) must skip the
/// bootstrap entirely — no `git checkout`, no `clear`, no noise.
#[test]
fn branch_bootstrap_returns_none_when_branch_absent() {
    assert!(branch_bootstrap_line(&None).is_none());
}

/// Empty / whitespace-only strings are treated as absent so a stray
/// `info.branch === ""` upstream never injects an empty-arg command.
#[test]
fn branch_bootstrap_returns_none_for_blank_branch() {
    assert!(branch_bootstrap_line(&Some(String::new())).is_none());
    assert!(branch_bootstrap_line(&Some("   ".to_string())).is_none());
}

/// A well-formed branch produces a `checkout || switch` line and
/// always ends with `clear\n` so the prompt lands on the new branch.
#[test]
fn branch_bootstrap_emits_checkout_then_switch_with_clear() {
    let line = branch_bootstrap_line(&Some("demeteo/features/abc".into()))
        .expect("bootstrap must be Some");
    assert!(
        line.starts_with("git checkout demeteo/features/abc"),
        "unexpected line: {line:?}"
    );
    assert!(
        line.contains("|| git switch demeteo/features/abc"),
        "missing switch fallback: {line:?}"
    );
    assert!(
        line.trim_end().ends_with("clear"),
        "missing clear: {line:?}"
    );
    assert!(
        line.ends_with('\n'),
        "must terminate with newline: {line:?}"
    );
}

/// Branch names containing shell metacharacters (`;`, `$`, quotes)
/// must be shell-escaped so a malicious / malformed feature id cannot
/// inject extra commands. The escape function itself is unit-tested
/// in `shared/shell.rs`; this test guards the wiring here.
#[test]
fn branch_bootstrap_escapes_shell_metacharacters() {
    let line =
        branch_bootstrap_line(&Some("evil;rm -rf /".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("'evil;rm -rf /'"),
        "metachars must be wrapped in single quotes: {line:?}"
    );
    // The unescaped form must NOT appear — that would be the
    // command-injection vector.
    assert!(
        !line.contains(" checkout evil;rm"),
        "unescaped branch leaked into command: {line:?}"
    );
}

/// A `branch` with a stray single quote is the trickiest case: it
/// must be quoted and the inner `'` escaped via the standard
/// `'\''` POSIX trick.
#[test]
fn branch_bootstrap_handles_inner_single_quote() {
    let line = branch_bootstrap_line(&Some("feat'bad".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("'feat'\\''bad'"),
        "inner single quote must be escaped: {line:?}"
    );
}

/// Surrounding whitespace is trimmed so `"  main  "` (e.g. from a UI
/// input) doesn't produce `git checkout   main` with extra spaces
/// that git refuses.
#[test]
fn branch_bootstrap_trims_surrounding_whitespace() {
    let line = branch_bootstrap_line(&Some("  feat/x  ".into())).expect("bootstrap must be Some");
    assert!(
        line.contains(" checkout feat/x "),
        "branch not trimmed: {line:?}"
    );
}
