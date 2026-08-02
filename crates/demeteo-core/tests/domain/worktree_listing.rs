//! The porcelain split that two adapter state machines used to make privately,
//! now reachable without a transport under it. `super` is
//! `crate::domain::worktree_listing`.

use super::*;

/// Git's own shape: blocks separated by a blank line, primary first.
fn porcelain(blocks: &[&str]) -> String {
    format!("{}\n", blocks.join("\n\n"))
}

fn entry(path: &str, branch: Option<&str>, is_locked: bool) -> WorktreeInfo {
    WorktreeInfo {
        path: path.to_string(),
        branch: branch.map(str::to_string),
        is_locked,
    }
}

// ── Which entry is the main checkout ─────────────────────────────────────────

#[test]
fn entry_zero_is_the_primary_and_never_joins_the_linked_entries() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nHEAD aaa\nbranch refs/heads/main",
        "worktree /repos/app_wt_s-1\nHEAD bbb\nbranch refs/heads/feat_subtask_s-1",
        "worktree /wt/terminal\nHEAD ccc\nbranch refs/heads/terminal/one",
    ]));

    assert_eq!(
        listing.primary,
        Some(entry("/repos/app", Some("main"), false))
    );
    assert_eq!(
        listing.linked,
        vec![
            entry("/repos/app_wt_s-1", Some("feat_subtask_s-1"), false),
            entry("/wt/terminal", Some("terminal/one"), false),
        ]
    );
}

#[test]
fn a_listing_with_only_a_primary_has_no_linked_entries() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nHEAD aaa\nbranch refs/heads/main",
    ]));

    assert_eq!(
        listing.primary.as_ref().map(|w| w.path.as_str()),
        Some("/repos/app")
    );
    assert!(
        listing.linked.is_empty(),
        "a repository with no linked worktrees must not report its own checkout as one"
    );
}

#[test]
fn every_entry_including_the_primary_is_reachable_in_gits_order() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nbranch refs/heads/main",
        "worktree /wt/one\nbranch refs/heads/one",
    ]));

    assert_eq!(
        listing.all().map(|w| w.path.as_str()).collect::<Vec<_>>(),
        ["/repos/app", "/wt/one"],
        "merge-back hunts a branch that is often checked out in the main repo"
    );
}

// ── What a block can carry ───────────────────────────────────────────────────

#[test]
fn a_detached_entry_carries_no_branch_and_is_still_listed() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nbranch refs/heads/main",
        "worktree /wt/detached\nHEAD bbb\ndetached",
    ]));

    assert_eq!(listing.linked, vec![entry("/wt/detached", None, false)]);
}

#[test]
fn a_locked_entry_does_not_leak_its_flag_to_the_next_entry() {
    let listing = parse(&porcelain(&[
        "worktree /repos/app\nbranch refs/heads/main",
        "worktree /wt/locked\nbranch refs/heads/locked\nlocked in use",
        "worktree /wt/open\nbranch refs/heads/open",
    ]));

    assert_eq!(
        listing.linked,
        vec![
            entry("/wt/locked", Some("locked"), true),
            entry("/wt/open", Some("open"), false),
        ]
    );
}

#[test]
fn a_final_block_not_terminated_by_a_blank_line_is_still_flushed() {
    let listing = parse(
        "worktree /repos/app\nbranch refs/heads/main\n\nworktree /wt/last\nbranch refs/heads/last",
    );

    assert_eq!(
        listing.linked,
        vec![entry("/wt/last", Some("last"), false)],
        "git does not always end its output with a blank line"
    );
}

#[test]
fn a_path_ending_in_whitespace_is_reported_as_git_spelled_it() {
    let listing = parse("worktree /repos/app\nbranch refs/heads/main\n\nworktree /wt/trailing \nbranch refs/heads/odd\n");

    assert_eq!(
        listing.linked.first().map(|w| w.path.as_str()),
        Some("/wt/trailing "),
        "trimming a path corrupts the one directory name that needs it verbatim"
    );
}
