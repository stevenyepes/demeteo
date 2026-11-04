// Tests extracted from `crates/demeteo-runner/src/git_askpass.rs`
// (mirrored-tests convention). `super` = that module.

use super::{ensure_askpass_script, git_command};
use demeteo_core::test_dir::TestDir;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

const QUERY: &str = "protocol=https\nhost=github.com\nusername=x-access-token\n\n";

/// A host gitconfig whose helper answers with its own password and records
/// every operation git sends it, `get` and `store` alike.
fn hostile_global_config(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let calls = dir.join("helper-calls");
    let config = dir.join("gitconfig");
    let helper = format!(
        "!f() {{ echo \"$1\" >> '{}'; test \"$1\" = get && printf 'password=from-host-helper\\n'; return 0; }}; f",
        calls.display()
    );
    std::fs::write(&config, format!("[credential]\n\thelper = \"{helper}\"\n")).unwrap();
    (config, calls)
}

fn run(mut cmd: Command, config: &Path, stdin: &str) -> String {
    cmd.env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_run_token_is_neither_answered_nor_stored_by_the_hosts_own_helper() {
    let dir = TestDir::new("git-askpass-helper");
    let askpass = ensure_askpass_script(dir.path()).unwrap();
    let (config, calls) = hostile_global_config(dir.path());

    let fill = git_command(
        &askpass,
        &["credential".into(), "fill".into()],
        Some("run-token"),
    );
    let answer = run(fill, &config, QUERY);
    assert!(answer.contains("password=run-token\n"), "{answer}");

    // What git does after a push it authenticated: `store` to every helper.
    let approve = git_command(
        &askpass,
        &["credential".into(), "approve".into()],
        Some("run-token"),
    );
    run(
        approve,
        &config,
        &format!("{}password=run-token\n\n", QUERY.trim_end()),
    );

    assert!(
        !calls.exists(),
        "the host's helper was asked: {}",
        std::fs::read_to_string(&calls).unwrap_or_default()
    );
}
