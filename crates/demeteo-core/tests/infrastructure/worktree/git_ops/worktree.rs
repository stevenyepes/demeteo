// Tests extracted from `crates/demeteo-core/src/adapters/worktree/git_ops/worktree.rs` (mirrored-tests convention). `super` = that module.

use super::link_dependency_caches_cmd;
use crate::paths::feature_cache_dir;

#[test]
fn command_iterates_every_known_cache_dir() {
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", "/repo_cache_feature-a");
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
    // These paths contain only shell-safe characters, so `shell_escape_posix`
    // leaves them bare (no quoting needed).
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", "/repo_cache_feature-a");
    assert!(cmd.contains("check-ignore -q"));
    assert!(cmd.contains("[ -e /repo/\"$d\" ]"));
    assert!(cmd.contains("[ ! -e /repo_wt_1/\"$d\" ]"));
}

/// The worktree must symlink into *this feature's* cache root, never straight
/// into the primary checkout. Linking to `{repo}/node_modules` is what let one
/// feature's install overwrite another's — and, worse, let one feature's build
/// output decide another feature's harness verdict.
#[test]
fn worktree_links_into_the_feature_cache_not_the_shared_primary() {
    let cache = feature_cache_dir("/repo", "feature/login");
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", &cache);

    assert!(
        cmd.contains("ln -sfn /repo_cache_feature-login/\"$d\" /repo_wt_1/\"$d\""),
        "worktree must link into the feature's own cache root: {cmd}"
    );
    assert!(
        !cmd.contains("ln -sfn /repo/\"$d\""),
        "worktree must NOT link straight at the shared primary checkout: {cmd}"
    );
}

/// Seeding must be a copy (ideally a copy-on-write clone), never a hardlink: a
/// tool that rewrites a file in place would write *through* a hardlink into
/// every other feature's tree, reintroducing the very bug this replaces.
#[test]
fn seeds_the_feature_cache_by_copy_preferring_copy_on_write() {
    let cmd = link_dependency_caches_cmd("/repo", "/repo_wt_1", "/repo_cache_feature-a");
    // APFS clonefile, then btrfs/xfs reflink, then a plain copy.
    assert!(cmd.contains("cp -cR"), "{cmd}");
    assert!(cmd.contains("--reflink=auto"), "{cmd}");
    assert!(
        !cmd.contains("cp -al") && !cmd.contains("ln /repo"),
        "seeding must never hardlink: {cmd}"
    );
    // Seed once — a feature's later steps reuse the cache they already have.
    assert!(
        cmd.contains("[ ! -e /repo_cache_feature-a/\"$d\" ]"),
        "{cmd}"
    );
}

#[test]
fn paths_with_special_chars_are_escaped() {
    let cmd = link_dependency_caches_cmd(
        "/repos/my repo",
        "/repos/my repo_wt_1",
        "/repos/my repo_cache_f",
    );
    assert!(cmd.contains("'/repos/my repo'"));
    assert!(cmd.contains("'/repos/my repo_wt_1'"));
    assert!(cmd.contains("'/repos/my repo_cache_f'"));
}

/// The cache root is keyed by feature branch, so two features on one repo can
/// never resolve to the same directory — that is the whole isolation property.
#[test]
fn feature_cache_dirs_are_distinct_per_feature_and_slash_free() {
    let a = feature_cache_dir("/repo", "feature/login");
    let b = feature_cache_dir("/repo", "feature/checkout");
    assert_ne!(a, b);
    assert_eq!(a, "/repo_cache_feature-login");
    // A raw `/` would nest the cache under a `feature/` directory instead of
    // sitting alongside the repo.
    assert!(!a.trim_start_matches("/repo").contains('/'), "{a}");
}
