use super::{agent_kind_for_binary, detect_agent_in_command};

#[cfg(target_os = "windows")]
use super::ProcessTree;

#[test]
fn agent_kind_maps_known_binaries() {
    assert_eq!(agent_kind_for_binary("claude"), Some("claude-code"));
    assert_eq!(agent_kind_for_binary("opencode"), Some("opencode"));
    assert_eq!(agent_kind_for_binary("codex"), Some("codex"));
    assert_eq!(agent_kind_for_binary("hermes"), Some("hermes"));
    assert_eq!(agent_kind_for_binary("pi"), Some("pi"));
    assert_eq!(agent_kind_for_binary("bash"), None);
    assert_eq!(agent_kind_for_binary("node"), None);
}

#[test]
fn detect_agent_matches_native_launcher() {
    assert_eq!(detect_agent_in_command("claude"), Some("claude-code"));
    assert_eq!(
        detect_agent_in_command("/opt/homebrew/bin/opencode --resume"),
        Some("opencode")
    );
    assert_eq!(detect_agent_in_command("pi --mode json"), Some("pi"));
}

#[test]
fn detect_agent_matches_script_token() {
    assert_eq!(
        detect_agent_in_command("node /Users/x/.bin/claude serve"),
        Some("claude-code")
    );
    assert_eq!(
        detect_agent_in_command("node /Users/x/tools/codex.js"),
        Some("codex")
    );
}

#[test]
fn detect_agent_ignores_non_agents() {
    assert_eq!(detect_agent_in_command("-zsh"), None);
    assert_eq!(detect_agent_in_command("vim notes/claude.txt"), None);
    assert_eq!(detect_agent_in_command("git -C /src/pi-mono status"), None);
    assert_eq!(detect_agent_in_command(""), None);
}

/// `pi` is a word before it is a binary, and the scan reads every token, not
/// just argv[0] — so it is only an agent where a binary can stand.
#[test]
fn detect_agent_ignores_a_bare_pi_argument() {
    assert_eq!(detect_agent_in_command("cargo run -p pi"), None);
    assert_eq!(detect_agent_in_command("git add pi"), None);
    assert_eq!(
        detect_agent_in_command("node /Users/x/.bin/pi --mode json"),
        Some("pi")
    );
}

#[test]
fn detect_agent_matches_windows_launchers() {
    assert_eq!(
        detect_agent_in_command("C:/Users/x/claude.cmd --resume"),
        Some("claude-code")
    );
    assert_eq!(
        detect_agent_in_command("C:\\Users\\x\\AppData\\claude.exe"),
        Some("claude-code")
    );
    assert_eq!(
        detect_agent_in_command("C:\\tools\\codex.EXE serve"),
        Some("codex")
    );
    assert_eq!(
        detect_agent_in_command("opencode.bat --version"),
        Some("opencode")
    );
    assert_eq!(detect_agent_in_command("C:\\notes\\claude.txt"), None);
}

#[cfg(target_os = "windows")]
#[test]
fn process_tree_capture_does_not_panic_on_windows() {
    let _ = ProcessTree::capture();
}
