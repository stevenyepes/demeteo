//! `super` is `crate::domain::workspace_fingerprint`.

use super::*;
use crate::domain::worktree_listing::parse;

const TIP: &str = "0123456789abcdef0123456789abcdef01234567";

fn porcelain(blocks: &[&str]) -> String {
    format!("{}\n", blocks.join("\n\n"))
}

// ── Which checkouts belong to the feature ────────────────────────────────────

#[test]
fn the_primary_counts_when_it_holds_the_feature_branch() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nHEAD aaa\nbranch refs/heads/demeteo/features/f-1",
        "worktree /repos/app_wt_other\nHEAD bbb\nbranch refs/heads/demeteo/features/f-2",
    ]));

    assert_eq!(
        feature_checkouts(&listing, "demeteo/features/f-1"),
        vec!["/repos/app"]
    );
}

#[test]
fn the_primary_on_another_features_branch_is_not_this_features_workspace() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nHEAD aaa\nbranch refs/heads/demeteo/features/f-2",
    ]));

    assert!(feature_checkouts(&listing, "demeteo/features/f-1").is_empty());
}

#[test]
fn subtask_worktrees_count_but_a_longer_feature_id_does_not() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nHEAD aaa\nbranch refs/heads/main",
        "worktree /repos/app_wt_f-1-step-s\nHEAD bbb\nbranch refs/heads/demeteo/features/f-1_subtask_f-1-step-s",
        "worktree /repos/app_wt_f-12-step-s\nHEAD ccc\nbranch refs/heads/demeteo/features/f-12_subtask_f-12-step-s",
        "worktree /repos/app_wt_detached\nHEAD ddd\ndetached",
    ]));

    assert_eq!(
        feature_checkouts(&listing, "demeteo/features/f-1"),
        vec!["/repos/app_wt_f-1-step-s"]
    );
}

// ── The rendered value ───────────────────────────────────────────────────────

#[test]
fn renders_the_tip_and_the_dirty_bit_under_the_versioned_scheme() {
    assert_eq!(
        render(TIP, false).as_deref(),
        Some(&*format!("v2:{TIP}:clean"))
    );
    assert_eq!(
        render(&format!("{TIP}\n"), true).as_deref(),
        Some(&*format!("v2:{TIP}:dirty"))
    );
}

#[test]
fn a_sha256_object_id_is_a_tip_too() {
    let tip = "a".repeat(64);
    assert!(render(&tip, false).is_some());
}

#[test]
fn anything_but_an_object_id_is_unknown() {
    assert_eq!(render("", false), None);
    assert_eq!(render("fatal: not a git repository", false), None);
    assert_eq!(render(&TIP[..39], false), None);
    assert_eq!(render(&"g".repeat(40), false), None);
}

// ── Comparing against rows already on disk ───────────────────────────────────

#[test]
fn only_a_value_this_scheme_rendered_is_comparable() {
    let current = render(TIP, true).expect("renders");
    assert!(is_comparable(&current));
    // What the first scheme recorded: HEAD of the shared clone, unversioned.
    assert!(!is_comparable(&format!("{TIP}:clean")));
    assert!(!is_comparable("unknown"));
}
