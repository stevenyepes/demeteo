// Tests extracted from `crates/demeteo-core/src/application/ask/path_containment.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn a_plain_relative_path_resolves_under_the_root() {
    let resolved = resolve_within_root("/repo/worktree", "src/main.rs")
        .expect("a plain relative path stays within the root");
    assert_eq!(resolved, std::path::Path::new("/repo/worktree/src/main.rs"));
}

#[test]
fn an_absolute_path_is_rejected() {
    assert_eq!(resolve_within_root("/repo/worktree", "/etc/hostname"), None);
}

#[test]
fn a_parent_dir_that_walks_back_past_the_root_is_rejected() {
    assert_eq!(
        resolve_within_root("/repo/worktree", "../../etc/hostname"),
        None
    );
}

#[test]
fn a_parent_dir_that_stays_within_the_root_resolves_like_the_plain_path() {
    let via_parent = resolve_within_root("/repo/worktree", "a/../b")
        .expect("a `..` that never crosses the root's boundary is still contained");
    let plain = resolve_within_root("/repo/worktree", "b")
        .expect("the equivalent plain path resolves the same way");
    assert_eq!(via_parent, plain);
}
