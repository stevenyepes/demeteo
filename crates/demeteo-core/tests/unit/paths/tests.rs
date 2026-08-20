// Tests extracted from `crates/demeteo-core/src/paths.rs` (mirrored-tests convention). `super` = that module.

use super::*;

#[test]
fn repo_name_from_path_handles_typical_inputs() {
    assert_eq!(repo_name_from_path("prototype/spectacular"), "spectacular");
    assert_eq!(repo_name_from_path("spectacular"), "spectacular");
    assert_eq!(repo_name_from_path("a/b/c/d"), "d");
    assert_eq!(repo_name_from_path("a/b/"), "b");
}

/// A branch is a path segment here. The previous spelling replaced only `/`,
/// so every other character a ref allows — `:`, `*`, a backslash — reached the
/// filesystem, and the flag is passed rather than read so both answers are
/// reachable from a Linux CI runner.
#[test]
fn a_sync_worktree_keeps_the_branch_readable_off_windows() {
    assert_eq!(
        sync_worktree_dir("/repos/demeteo", "feature/f-1", false),
        "/repos/demeteo_wt_sync_feature-f-1"
    );
    assert_eq!(
        sync_worktree_dir("/repos/demeteo", "fix/UI:2*3", false),
        "/repos/demeteo_wt_sync_fix-UI-2-3"
    );
}

/// `CreateProcessW` rejects a working directory past `MAX_PATH` whatever
/// `core.longpaths` says, and a branch name is unbounded — so on a
/// Windows-local target the branch folds to a fixed-width segment, as the step
/// worktrees already do.
#[test]
fn a_sync_worktree_on_a_windows_host_folds_the_branch_to_a_fixed_width() {
    let long = format!("feature/{}", "x".repeat(400));
    let dir = sync_worktree_dir("C:/repos/demeteo", &long, true);
    assert_eq!(
        dir.len(),
        "C:/repos/demeteo_wt_sync_".len() + SHORT_SEGMENT_LEN
    );
    assert_ne!(
        dir,
        sync_worktree_dir("C:/repos/demeteo", "feature/other", true)
    );
}

/// Provisioning, the stale-worktree scan and the teardown all key on the
/// `_wt_sync` infix; a helper that dropped it would leave live worktrees that
/// the scan no longer finds.
#[test]
fn a_sync_worktree_keeps_the_infix_the_stale_scan_matches_on() {
    assert!(sync_worktree_dir("/r/d", "b", false).contains("_wt_sync"));
    assert!(sync_worktree_dir("/r/d", "b", true).contains("_wt_sync"));
}
