use std::collections::HashMap;

use crate::domain::agent_env::{agent_git_config_env, inherited_agent_env};
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
    for desktop in Platform::ALL {
        assert!(
            inherited_agent_env(Some(desktop), Some(Platform::Windows), git_bash_desktop)
                .is_empty(),
            "a Windows agent must not be told the desktop's shell or temp directory, \
             whatever the desktop is ({desktop})"
        );
    }
}

/// The same false claim pointed the other way: the values are the desktop's,
/// and a Windows desktop driving a Linux remote has neither to give.
#[test]
fn a_windows_desktop_forwards_nothing_to_a_posix_agent() {
    for target in [Platform::Linux, Platform::MacOS] {
        assert!(
            inherited_agent_env(Some(Platform::Windows), Some(target), git_bash_desktop).is_empty(),
            "a Git Bash desktop's `/usr/bin/bash` names no file on the {target} remote"
        );
    }
}

#[test]
fn a_posix_agent_inherits_both_unchanged() {
    for desktop in [Platform::Linux, Platform::MacOS] {
        for target in [Platform::Linux, Platform::MacOS] {
            let env = inherited_agent_env(Some(desktop), Some(target), git_bash_desktop);
            assert_eq!(names(&env), ["SHELL", "TMPDIR"], "{desktop} → {target}");
            assert_eq!(env[0].1, "/usr/bin/bash");
            assert_eq!(env[1].1, "/tmp");
        }
    }
}

#[test]
fn an_unnameable_platform_is_not_assumed_posix() {
    assert!(inherited_agent_env(Some(Platform::Linux), None, git_bash_desktop).is_empty());
    assert!(inherited_agent_env(None, Some(Platform::Linux), git_bash_desktop).is_empty());
    assert!(inherited_agent_env(None, None, git_bash_desktop).is_empty());
}

#[test]
fn a_variable_the_desktop_never_set_is_not_forged_empty() {
    assert!(inherited_agent_env(Some(Platform::Linux), Some(Platform::Linux), |_| None).is_empty());
}

#[test]
fn nothing_beyond_the_two_is_taken_from_the_desktop() {
    let env = inherited_agent_env(Some(Platform::Linux), Some(Platform::Linux), |name| {
        Some(format!("value-of-{name}"))
    });
    assert_eq!(names(&env), ["SHELL", "TMPDIR"]);
}

/// The pin applies where the declaration it defends is load-bearing, and
/// nowhere else. Both halves matter: dropped on Windows, the Windows block
/// claims a shell the harness may not have; kept off Windows, Demeteo removes a
/// tool the user opted into on a platform with no claim to defend.
#[test]
fn a_shell_pin_reaches_windows_and_no_other_platform() {
    use crate::domain::agent_env::pinned_shell_env;

    const PINS: &[(&str, &str)] = &[("CLAUDE_CODE_USE_POWERSHELL_TOOL", "0")];

    assert_eq!(pinned_shell_env(Some(Platform::Windows), PINS), PINS);
    for elsewhere in [Some(Platform::Linux), Some(Platform::MacOS), None] {
        assert!(
            pinned_shell_env(elsewhere, PINS).is_empty(),
            "{elsewhere:?} has no declaration to defend"
        );
    }
}

/// What git itself would read out of `env`: the `GIT_CONFIG_*` block walked
/// the way git walks it, so a gap or an unread index fails here the way it
/// would fail there.
fn git_config_of(env: &HashMap<String, String>) -> Vec<(String, String)> {
    let count: usize = env
        .get("GIT_CONFIG_COUNT")
        .map(|count| count.parse().expect("GIT_CONFIG_COUNT is a number"))
        .unwrap_or(0);
    (0..count)
        .map(|i| {
            let key = env.get(&format!("GIT_CONFIG_KEY_{i}"));
            let value = env.get(&format!("GIT_CONFIG_VALUE_{i}"));
            match (key, value) {
                (Some(key), Some(value)) => (key.clone(), value.clone()),
                _ => panic!("git fatals on a missing GIT_CONFIG_KEY/VALUE_{i}"),
            }
        })
        .collect()
}

fn merged(agent_kind: &str, mut env: HashMap<String, String>) -> HashMap<String, String> {
    let git = agent_git_config_env(agent_kind, &env);
    env.extend(git);
    env
}

fn pairs(entries: &[(&str, &str)]) -> Vec<(String, String)> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn an_agent_commits_as_demeteo_agent_and_never_signs() {
    assert_eq!(
        git_config_of(&merged("claude-code", HashMap::new())),
        pairs(&[
            ("user.name", "demeteo-agent (claude-code)"),
            ("user.email", "demeteo-agent@local"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ])
    );
}

#[test]
fn the_committing_agent_is_named_in_the_identity() {
    for kind in ["opencode", "codex", "hermes"] {
        let config = git_config_of(&merged(kind, HashMap::new()));
        assert!(
            config.contains(&("user.name".to_string(), format!("demeteo-agent ({kind})"))),
            "{kind}: {config:?}"
        );
    }
}

/// A block the caller already built keeps its indices; ours is appended.
#[test]
fn an_existing_git_config_block_is_extended_not_overwritten() {
    let env: HashMap<String, String> = pairs(&[
        ("GIT_CONFIG_COUNT", "2"),
        ("GIT_CONFIG_KEY_0", "safe.directory"),
        ("GIT_CONFIG_VALUE_0", "*"),
        ("GIT_CONFIG_KEY_1", "core.autocrlf"),
        ("GIT_CONFIG_VALUE_1", "false"),
    ])
    .into_iter()
    .collect();

    let config = git_config_of(&merged("pi", env));

    assert_eq!(config.len(), 6, "{config:?}");
    assert_eq!(
        config[..3],
        pairs(&[
            ("safe.directory", "*"),
            ("core.autocrlf", "false"),
            ("user.name", "demeteo-agent (pi)"),
        ])
    );
}
