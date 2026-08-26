// Tests extracted from `crates/demeteo-core/src/domain/upstream_feature.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn counted(behind: u64, ahead: u64) -> BranchDivergence {
    BranchDivergence {
        behind: Some(behind),
        ahead: Some(ahead),
    }
}

/// The two ordinary shapes, which have to be told apart from each other and
/// from the third: a branch level with origin and a branch carrying unpushed
/// work are both branches origin has nothing to add to.
#[test]
fn a_branch_origin_cannot_add_to_is_left_alone() {
    assert_eq!(reconcile(counted(0, 0)), FeatureUpstream::Current);
    assert_eq!(reconcile(counted(0, 7)), FeatureUpstream::Current);
}

#[test]
fn a_branch_origin_has_moved_past_is_fast_forwarded() {
    assert_eq!(reconcile(counted(3, 0)), FeatureUpstream::FastForward);
}

/// The incident: origin held a hand-written fix this clone had never seen, the
/// sync merged the base into the branch without it, and the merge commit read
/// as a clean merge while being a revert of that fix.
#[test]
fn a_branch_that_moved_on_both_sides_is_refused_with_both_counts() {
    assert_eq!(
        reconcile(counted(1, 2)),
        FeatureUpstream::Diverged {
            ahead: 2,
            behind: 1
        }
    );
}

/// The same rule the base-branch short-circuit applies, on the other side of
/// the merge: a `rev-list` that did not answer is not a branch with nothing to
/// pull. Reading it as `Current` restores exactly the silent skip this module
/// exists to remove, where reading it as a fast-forward costs at most a
/// `merge --ff-only` that answers "Already up to date".
#[test]
fn an_unmeasured_branch_is_never_called_current() {
    for divergence in [
        BranchDivergence::unknown(),
        BranchDivergence {
            behind: None,
            ahead: Some(0),
        },
        BranchDivergence {
            behind: Some(4),
            ahead: None,
        },
    ] {
        assert_eq!(
            reconcile(divergence),
            FeatureUpstream::FastForward,
            "{divergence:?} was read as an answer"
        );
    }
}

/// Both refusals have to say which branch, and what the user is expected to do
/// about it — the banner shows this string and nothing else, and "sync again"
/// on its own is an instruction to repeat the thing that just refused.
#[test]
fn every_refusal_names_the_branch_and_the_move() {
    let counted = diverged_refusal("demeteo/features/f-1", "master", 2, 1);
    assert!(counted.contains("demeteo/features/f-1"), "{counted}");
    assert!(counted.contains("origin/master"), "{counted}");
    assert!(counted.contains('1') && counted.contains('2'), "{counted}");
    assert!(
        counted.contains("Reconcile the branch yourself"),
        "{counted}"
    );

    let refused = unmergeable_refusal(
        "demeteo/features/f-1",
        "master",
        "fatal: Not possible to fast-forward, aborting.",
    );
    assert!(refused.contains("demeteo/features/f-1"), "{refused}");
    assert!(
        refused.contains("fatal: Not possible to fast-forward, aborting."),
        "git's own words are the only account of what it refused: {refused}"
    );
    assert!(
        refused.contains("Reconcile the branch yourself"),
        "{refused}"
    );
}
