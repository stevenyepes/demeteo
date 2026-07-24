use super::{
    branch_bootstrap_line, branch_bootstrap_line_posix, cmd_double_quote, select_local_shell,
};

#[cfg(not(target_os = "windows"))]
#[test]
fn select_local_shell_returns_non_empty_posix_shell() {
    let shell = select_local_shell();
    assert!(!shell.is_empty(), "shell must not be empty: {shell:?}");
    assert!(
        shell.starts_with('/'),
        "expected an absolute POSIX shell path: {shell:?}"
    );
}

#[cfg(target_os = "windows")]
#[test]
fn select_local_shell_returns_windows_command_processor() {
    let shell = select_local_shell();
    let expected = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".to_string());
    assert_eq!(shell, expected, "Windows shell must be COMSPEC/cmd.exe");
    assert!(
        !shell.contains("/bin/bash"),
        "Windows must never select /bin/bash: {shell:?}"
    );
}

#[test]
fn branch_bootstrap_returns_none_when_branch_absent() {
    assert!(branch_bootstrap_line(&None).is_none());
}

#[test]
fn branch_bootstrap_returns_none_for_blank_branch() {
    assert!(branch_bootstrap_line(&Some(String::new())).is_none());
    assert!(branch_bootstrap_line(&Some("   ".to_string())).is_none());
}

#[cfg(not(target_os = "windows"))]
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

#[cfg(not(target_os = "windows"))]
#[test]
fn branch_bootstrap_escapes_shell_metacharacters() {
    let line =
        branch_bootstrap_line(&Some("evil;rm -rf /".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("'evil;rm -rf /'"),
        "metachars must be wrapped in single quotes: {line:?}"
    );
    assert!(
        !line.contains(" checkout evil;rm"),
        "unescaped branch leaked into command: {line:?}"
    );
}

#[cfg(not(target_os = "windows"))]
#[test]
fn branch_bootstrap_handles_inner_single_quote() {
    let line = branch_bootstrap_line(&Some("feat'bad".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("'feat'\\''bad'"),
        "inner single quote must be escaped: {line:?}"
    );
}

#[cfg(target_os = "windows")]
#[test]
fn branch_bootstrap_emits_cmd_checkout_then_switch_with_cls() {
    let line = branch_bootstrap_line(&Some("demeteo/features/abc".into()))
        .expect("bootstrap must be Some");
    assert!(
        line.starts_with("git checkout \"demeteo/features/abc\" 2>nul"),
        "unexpected line: {line:?}"
    );
    assert!(
        line.contains("|| git switch \"demeteo/features/abc\" 2>nul"),
        "missing switch fallback: {line:?}"
    );
    assert!(line.contains("2>nul"), "must redirect to nul: {line:?}");
    assert!(
        line.contains("& cls"),
        "must chain a cmd screen clear: {line:?}"
    );
    assert!(
        line.trim_end().ends_with("cls"),
        "must clear via cls: {line:?}"
    );
    assert!(
        !line.contains("2>/dev/null") && !line.contains("clear\n"),
        "POSIX syntax leaked into cmd variant: {line:?}"
    );
    assert!(
        line.ends_with("\r\n"),
        "cmd variant must terminate with CRLF: {line:?}"
    );
}

#[cfg(target_os = "windows")]
#[test]
fn branch_bootstrap_cmd_escapes_metacharacters() {
    let line =
        branch_bootstrap_line(&Some("evil&del /q *".into())).expect("bootstrap must be Some");
    assert!(
        line.contains("\"evil^&del /q *\""),
        "cmd metachars must be quoted and caret-escaped: {line:?}"
    );
    assert!(
        !line.contains("checkout \"evil&del"),
        "unescaped `&` leaked into command: {line:?}"
    );
}

#[test]
fn branch_bootstrap_trims_surrounding_whitespace() {
    let line = branch_bootstrap_line(&Some("  feat/x  ".into())).expect("bootstrap must be Some");
    assert!(line.contains("feat/x"), "trimmed branch missing: {line:?}");
    assert!(
        !line.contains("  feat/x"),
        "leading whitespace not trimmed: {line:?}"
    );
    assert!(
        !line.contains("feat/x  "),
        "trailing whitespace not trimmed: {line:?}"
    );
}

#[test]
fn branch_bootstrap_posix_is_posix_on_every_platform() {
    let line = branch_bootstrap_line_posix(&Some("demeteo/features/abc".into()))
        .expect("bootstrap must be Some");
    assert!(
        line.starts_with("git checkout demeteo/features/abc 2>/dev/null"),
        "SSH bootstrap must use POSIX /dev/null: {line:?}"
    );
    assert!(
        line.contains("|| git switch demeteo/features/abc 2>/dev/null"),
        "missing POSIX switch fallback: {line:?}"
    );
    assert!(
        line.trim_end().ends_with("clear"),
        "SSH bootstrap must clear via POSIX `clear`: {line:?}"
    );
    assert!(
        !line.contains("2>nul") && !line.contains("cls") && !line.contains('"'),
        "cmd.exe syntax leaked into the POSIX/remote bootstrap: {line:?}"
    );
    assert!(
        line.ends_with('\n') && !line.ends_with("\r\n"),
        "POSIX bootstrap must use a bare LF terminator: {line:?}"
    );
    assert!(branch_bootstrap_line_posix(&None).is_none());
    assert!(branch_bootstrap_line_posix(&Some("   ".to_string())).is_none());
}

#[test]
fn cmd_double_quote_neutralises_metacharacters() {
    assert_eq!(
        cmd_double_quote("demeteo/features/abc"),
        "\"demeteo/features/abc\""
    );

    assert_eq!(
        cmd_double_quote("a&b|c<d>e%f^g\"h"),
        "\"a^&b^|c^<d^>e^%f^^g^\"h\""
    );

    let escaped = cmd_double_quote("x\"&del /q *");
    assert_eq!(escaped, "\"x^\"^&del /q *\"");
    assert!(
        !escaped.contains("\"&del"),
        "unescaped break-out sequence survived: {escaped:?}"
    );
    assert!(
        escaped.starts_with('"') && escaped.ends_with('"'),
        "result must be wrapped in double quotes: {escaped:?}"
    );
}
