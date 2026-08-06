use crate::domain::agent_env::inherited_agent_env;
use crate::domain::models::Platform;

/// The environment the leak was actually found in: a desktop started from a
/// Git Bash terminal on Windows. Spelled as data rather than read from the
/// host, so the assertions mean the same thing on every machine that runs
/// them — a test consulting the real `std::env` would go vacuous wherever the
/// variable happens to be unset.
fn git_bash_desktop(name: &str) -> Option<String> {
    match name {
        "SHELL" => Some("/usr/bin/bash".to_string()),
        "TMPDIR" => Some("/tmp".to_string()),
        _ => None,
    }
}

fn names(env: &[(String, String)]) -> Vec<&str> {
    env.iter().map(|(name, _)| name.as_str()).collect()
}

#[test]
fn a_windows_agent_inherits_no_posix_claim() {
    assert!(
        inherited_agent_env(Some(Platform::Windows), git_bash_desktop).is_empty(),
        "a Windows agent must not be told the desktop's shell or temp directory"
    );
}

#[test]
fn a_posix_agent_inherits_both_unchanged() {
    for target in [Platform::Linux, Platform::MacOS] {
        let env = inherited_agent_env(Some(target), git_bash_desktop);
        assert_eq!(names(&env), ["SHELL", "TMPDIR"], "on {target}");
        assert_eq!(env[0].1, "/usr/bin/bash");
        assert_eq!(env[1].1, "/tmp");
    }
}

#[test]
fn an_unnameable_target_is_not_assumed_posix() {
    assert!(inherited_agent_env(None, git_bash_desktop).is_empty());
}

#[test]
fn a_variable_the_desktop_never_set_is_not_forged_empty() {
    assert!(inherited_agent_env(Some(Platform::Linux), |_| None).is_empty());
}

#[test]
fn nothing_beyond_the_two_is_taken_from_the_desktop() {
    let env = inherited_agent_env(Some(Platform::Linux), |name| {
        Some(format!("value-of-{name}"))
    });
    assert_eq!(names(&env), ["SHELL", "TMPDIR"]);
}
