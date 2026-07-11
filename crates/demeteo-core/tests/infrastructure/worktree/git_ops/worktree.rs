// Tests extracted from `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs` (mirrored-tests convention). `super` = that module.

use super::link_dependency_caches_cmd;

#[test]
fn command_iterates_every_known_cache_dir() {
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1");
    for dir in crate::paths::DEPENDENCY_CACHE_DIRS {
        assert!(
            cmd.contains(dir),
            "expected command to reference '{}': {}",
            dir,
            cmd
        );
    }
}

#[test]
fn command_gates_on_existence_and_check_ignore() {
    // `/repo` and `/repo_wt_1` contain only shell-safe characters, so
    // `shell_escape_posix` leaves them bare (no quoting needed).
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1");
    assert!(cmd.contains("check-ignore -q"));
    assert!(cmd.contains("[ -e /repo/\"$d\" ]"));
    assert!(cmd.contains("[ ! -e /repo_wt_1/\"$d\" ]"));
    assert!(cmd.contains("ln -sfn /repo/\"$d\" /repo_wt_1/\"$d\""));
}

#[test]
fn paths_with_special_chars_are_escaped() {
    let cmd = link_dependency_caches_cmd("/repos/my repo", "/repos/my repo_wt_1");
    assert!(cmd.contains("'/repos/my repo'"));
    assert!(cmd.contains("'/repos/my repo_wt_1'"));
}
