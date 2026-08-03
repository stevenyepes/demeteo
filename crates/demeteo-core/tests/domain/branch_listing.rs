//! The base-branch candidates a picker offers, reachable without a repository
//! under it. `super` is `crate::domain::branch_listing`.

use super::*;

fn option(name: &str, has_local: bool, has_remote: bool) -> BranchOption {
    BranchOption {
        name: name.to_string(),
        has_local,
        has_remote,
    }
}

fn names(refs: &str) -> Vec<String> {
    parse(refs).into_iter().map(|option| option.name).collect()
}

// ── Collapsing the two spellings of one branch ───────────────────────────────

#[test]
fn a_branch_present_locally_and_on_origin_is_one_candidate() {
    assert_eq!(
        parse("refs/heads/main\nrefs/remotes/origin/main\n"),
        vec![option("main", true, true)],
    );
}

#[test]
fn a_branch_only_on_origin_is_offered_and_marked_as_such() {
    assert_eq!(
        parse("refs/heads/main\nrefs/remotes/origin/main\nrefs/remotes/origin/release\n"),
        vec![option("main", true, true), option("release", false, true)],
        "a base never checked out locally is still a base worth cutting from"
    );
}

#[test]
fn a_branch_only_local_carries_no_remote_flag() {
    assert_eq!(
        parse("refs/heads/scratch\n"),
        vec![option("scratch", true, false)],
        "the flag is what tells a caller there is nothing on origin to refresh from"
    );
}

#[test]
fn a_slash_separated_branch_keeps_every_component_after_the_remote_prefix() {
    assert_eq!(
        parse("refs/heads/feature/deep/name\nrefs/remotes/origin/feature/deep/name\n"),
        vec![option("feature/deep/name", true, true)],
    );
}

// ── What is withheld ─────────────────────────────────────────────────────────

#[test]
fn the_origin_head_pointer_is_not_a_branch_anyone_means_to_name() {
    assert_eq!(
        names("refs/remotes/origin/HEAD\nrefs/remotes/origin/main\n"),
        ["main"],
    );
}

#[test]
fn a_subtask_branch_is_withheld_because_a_run_may_be_mid_write_in_it() {
    assert_eq!(
        names(
            "refs/heads/feat\nrefs/heads/feat_subtask_s-1\nrefs/remotes/origin/feat_subtask_s-1\n"
        ),
        ["feat"],
    );
}

#[test]
fn refs_outside_heads_and_origin_are_ignored() {
    assert_eq!(
        names("refs/tags/v1.0.0\nrefs/remotes/upstream/main\nrefs/stash\nrefs/heads/main\n\n   \n"),
        ["main"],
        "a tag or another remote is not a branch this repository can cut a worktree from"
    );
}

// ── Ordering ─────────────────────────────────────────────────────────────────

#[test]
fn candidates_come_out_sorted_by_the_collapsed_name() {
    assert_eq!(
        names(
            "refs/heads/zeta\nrefs/heads/alpha\nrefs/remotes/origin/beta\nrefs/remotes/origin/alpha\n"
        ),
        ["alpha", "beta", "zeta"],
        "git sorts by refname, so every local ref would otherwise precede every remote one"
    );
}
