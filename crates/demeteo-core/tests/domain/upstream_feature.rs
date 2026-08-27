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

const REWRITTEN: &str = "6b1f0a3c7d92e845b0c1f7a2d3e4b5c60718293a";
const MINE: &str = "0f2e4d6c8b0a1937556473829100aabbccddeeff";
const ALSO_MINE: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9001122334";

fn cherry(lines: &[String]) -> String {
    format!("{}\n", lines.join("\n"))
}

/// The rebase-elsewhere shape: origin holds the same changes under other shas,
/// which is the only reading under which throwing the local commits away is a
/// loss of nothing.
#[test]
fn a_branch_origin_rewrote_is_offered_a_reset() {
    let output = cherry(&[format!("- {REWRITTEN}"), format!("- {MINE}")]);
    assert_eq!(
        classify_divergence(Some(&output), 2),
        DivergenceMove::ResetOntoOrigin
    );
}

/// The shape the old refusal cost the most: two people working the same branch
/// from two clones. Neither side is upstream of the other and neither needs to
/// be dropped, so this is the arm a sync may take without asking.
#[test]
fn disjoint_work_on_both_sides_is_merged() {
    let output = cherry(&[format!("+ {MINE}"), format!("+ {ALSO_MINE}")]);
    assert_eq!(
        classify_divergence(Some(&output), 2),
        DivergenceMove::MergeOrigin
    );
}

/// Every shape that is not unanimous is a different way of not knowing, and
/// they all have to arrive at the same refusal. The ones that would cost a
/// branch are the reads that only look unanimous: nothing at all, for a branch
/// the counts called ahead, is a failed read rather than "all `-`", and a line
/// the parser cannot place beside lines it can is a partial read of an answer
/// whose missing half is the half that decides.
#[test]
fn a_divergence_git_cherry_cannot_settle_is_still_refused() {
    let mixed = cherry(&[format!("- {REWRITTEN}"), format!("+ {MINE}")]);
    let unmarked = cherry(&[MINE.to_string()]);
    let marked_nothing = cherry(&["-".to_string(), "+".to_string()]);
    let unknown_mark = cherry(&[format!("? {MINE}")]);
    let truncated_reset = cherry(&[format!("- {REWRITTEN}"), "error: short read".to_string()]);
    let truncated_merge = cherry(&[format!("+ {MINE}"), "error: short read".to_string()]);
    for output in [
        None,
        Some(""),
        Some("  \n\n\t\n"),
        Some(mixed.as_str()),
        Some(unmarked.as_str()),
        Some(marked_nothing.as_str()),
        Some(unknown_mark.as_str()),
        Some(truncated_reset.as_str()),
        Some(truncated_merge.as_str()),
        Some("fatal: bad revision 'origin/demeteo/features/f-1'"),
    ] {
        assert_eq!(
            classify_divergence(output, 2),
            DivergenceMove::Refuse,
            "{output:?} was read as an answer"
        );
    }
}

/// `git cherry` walks with `max_parents=1`, so a merge commit in the ahead set
/// is never printed — and the lines that remain then read as unanimous over a
/// history nothing looked at. The reset arm is the only one that can lose
/// anything, so it is the only one the count gates: a merge keeps both sides
/// whatever it did not see.
///
/// That git really does skip merges is a fixture's claim, not this one's
/// (`a_merge_commit_the_cherry_never_printed_is_not_a_reset`).
#[test]
fn a_cherry_shorter_than_the_ahead_count_is_not_unanimous() {
    let one_rewritten = cherry(&[format!("- {REWRITTEN}")]);
    assert_eq!(
        classify_divergence(Some(&one_rewritten), 1),
        DivergenceMove::ResetOntoOrigin,
        "one line for one commit ahead is the whole history"
    );
    for ahead in [2, 3, 40] {
        assert_eq!(
            classify_divergence(Some(&one_rewritten), ahead),
            DivergenceMove::Refuse,
            "one classified commit out of {ahead} was read as all of them"
        );
    }

    let one_mine = cherry(&[format!("+ {MINE}")]);
    for ahead in [1, 2, 40] {
        assert_eq!(
            classify_divergence(Some(&one_mine), ahead),
            DivergenceMove::MergeOrigin,
            "a merge over {ahead} commit(s) drops none of them"
        );
    }
}

fn branch() -> DivergedBranch<'static> {
    DivergedBranch {
        feature: "demeteo/features/f-1",
        base: "master",
        ahead: 2,
        behind: 1,
    }
}

/// Nobody pressed anything, which is every unattended sync: the answers are
/// [`classify_divergence`]'s, and the two that would pick a history stop.
#[test]
fn an_unpressed_divergence_takes_only_the_move_it_measured() {
    let disjoint = cherry(&[format!("+ {MINE}")]);
    // Two lines for the two commits `branch()` is ahead by: one `-` over a
    // branch counted two ahead is the merge-commit read the reset arm refuses.
    let rewritten = cherry(&[format!("- {REWRITTEN}"), format!("- {MINE}")]);
    let mixed = cherry(&[format!("- {REWRITTEN}"), format!("+ {MINE}")]);

    assert_eq!(
        divergence_move(branch(), None, Some(&disjoint)),
        Ok(DivergenceReconcile::MergeOrigin)
    );
    assert_eq!(
        divergence_move(branch(), None, Some(&rewritten)),
        Err(rewritten_refusal("demeteo/features/f-1", 2, 1))
    );
    assert_eq!(
        divergence_move(branch(), None, Some(&mixed)),
        Err(diverged_refusal("demeteo/features/f-1", "master", 2, 1))
    );
    assert_eq!(
        divergence_move(branch(), None, None),
        Err(diverged_refusal("demeteo/features/f-1", "master", 2, 1))
    );
}

/// A merge cannot drop either side, so there is no reading of the branch under
/// which having merged is the wrong thing to have done — including the readings
/// that refuse to say anything at all. The press that arrives over an
/// unreadable `git cherry` is the one this protects: refusing it would leave
/// the user with a branch nothing can reconcile and a button that does nothing.
#[test]
fn a_pressed_merge_is_taken_whatever_was_measured() {
    let rewritten = cherry(&[format!("- {REWRITTEN}")]);
    let mixed = cherry(&[format!("- {REWRITTEN}"), format!("+ {MINE}")]);
    for cherry_output in [
        None,
        Some(""),
        Some(rewritten.as_str()),
        Some(mixed.as_str()),
    ] {
        assert_eq!(
            divergence_move(
                branch(),
                Some(DivergenceReconcile::MergeOrigin),
                cherry_output
            ),
            Ok(DivergenceReconcile::MergeOrigin),
            "{cherry_output:?} withheld a merge"
        );
    }
}

/// The press the whole re-measurement exists for. The pane offered the reset
/// while origin carried every change the local commits make; by the time the
/// button was pressed it need not still, and the difference between the two
/// readings is a commit nobody has read being discarded.
#[test]
fn a_pressed_reset_is_taken_only_while_it_still_loses_nothing() {
    let rewritten = cherry(&[format!("- {REWRITTEN}"), format!("- {MINE}")]);
    assert_eq!(
        divergence_move(
            branch(),
            Some(DivergenceReconcile::ResetOntoOrigin),
            Some(&rewritten)
        ),
        Ok(DivergenceReconcile::ResetOntoOrigin)
    );

    let now_disjoint = cherry(&[format!("+ {MINE}")]);
    let now_mixed = cherry(&[format!("- {REWRITTEN}"), format!("+ {MINE}")]);
    for cherry_output in [
        None,
        Some(""),
        Some(now_disjoint.as_str()),
        Some(now_mixed.as_str()),
    ] {
        assert_eq!(
            divergence_move(
                branch(),
                Some(DivergenceReconcile::ResetOntoOrigin),
                cherry_output
            ),
            Err(stale_reset_refusal("demeteo/features/f-1", 2)),
            "{cherry_output:?} was reset on"
        );
    }
}

/// The refused reset is read by someone looking at the button they just
/// pressed, so it has to say that nothing happened and why — not the standing
/// refusal, which reads as if they had asked for something Demeteo never
/// offers.
#[test]
fn the_refused_reset_says_nothing_was_changed() {
    let stale = stale_reset_refusal("demeteo/features/f-1", 2);
    assert!(stale.contains("demeteo/features/f-1"), "{stale}");
    assert!(stale.contains("Nothing was changed"), "{stale}");
    assert!(!stale.contains("Reconcile the branch yourself"), "{stale}");
    assert!(!stale.contains("  "), "{stale}");
}

/// The wire spelling of both types, pinned here because nothing else can hold
/// it: `next_move` is read by a frontend union and `reconcile` is what the
/// reconcile IPC is *called* with, so a derive that renamed either one would
/// break the press and the offer with a green suite behind it.
///
/// `refuse` is deliberately not a value [`DivergenceReconcile`] accepts. It is
/// the measurement's non-answer, and a caller that could send it would be
/// asking for a move that does not exist.
#[test]
fn the_wire_spelling_is_pinned() {
    for (value, wire) in [
        (DivergenceMove::MergeOrigin, "\"merge_origin\""),
        (DivergenceMove::ResetOntoOrigin, "\"reset_onto_origin\""),
        (DivergenceMove::Refuse, "\"refuse\""),
    ] {
        assert_eq!(serde_json::to_string(&value).expect("serializes"), wire);
    }
    assert_eq!(
        serde_json::from_str::<DivergenceReconcile>("\"reset_onto_origin\"").expect("parses"),
        DivergenceReconcile::ResetOntoOrigin
    );
    assert!(serde_json::from_str::<DivergenceReconcile>("\"refuse\"").is_err());

    assert_eq!(
        serde_json::to_string(&crate::domain::models::FeatureDivergence {
            ahead: 2,
            behind: 1,
            next_move: DivergenceMove::ResetOntoOrigin,
        })
        .expect("serializes"),
        r#"{"ahead":2,"behind":1,"next_move":"reset_onto_origin"}"#
    );
}
