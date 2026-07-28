// Tests extracted from `crates/demeteo-core/src/adapters/step_executor/steps/sequence/mod.rs` (mirrored-tests convention). `super` = that module.

use super::*;

const ANCHOR: &str = "1111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222";

/// `git merge-base <anchor> <base>` printing the anchor itself is the only
/// evidence that the prefix already reached the feature branch.
#[test]
fn the_merge_base_being_the_anchor_means_merged() {
    assert!(anchor_is_merged(&format!("{ANCHOR}\n"), ANCHOR));
}

/// An *earlier* common ancestor means the anchor is off on the step branch:
/// the crash shape, and the only case that may reset a worktree.
#[test]
fn an_earlier_merge_base_means_the_prefix_is_stranded() {
    assert!(!anchor_is_merged(&format!("{OTHER}\n"), ANCHOR));
}

/// The failure this probe shape exists to prevent. `merge-base` answers on
/// stdout, so "git could not answer" arrives as an `Err` the caller turns
/// into a full re-run — never as output that reads like a verdict. Empty or
/// unparseable output must not be mistaken for a match either.
#[test]
fn no_answer_is_never_read_as_merged() {
    assert!(!anchor_is_merged("", ANCHOR));
    assert!(!anchor_is_merged("   \n", ANCHOR));
    assert!(!anchor_is_merged("fatal: not a git repository\n", ANCHOR));
}

/// Git prints lowercase hex; a checkpoint written from a differently-cased
/// source must still match rather than silently resolving to "stranded" and
/// resetting a worktree backwards over merged work.
#[test]
fn the_comparison_ignores_sha_case_and_whitespace() {
    assert!(anchor_is_merged(
        "  1111111111111111111111111111111111111111  \n",
        "1111111111111111111111111111111111111111"
    ));
    assert!(anchor_is_merged(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    ));
}

/// The rewind is only correct if it reproduces the row the attempt *read*.
/// Nothing to resume from means nothing to write back.
#[test]
fn rewinding_no_resume_clears_the_row() {
    let (ids, anchor, produced) = CheckpointResume::None.as_stored();
    assert!(ids.is_empty());
    assert_eq!(anchor, None);
    assert_eq!(produced, None);
}

/// A merged prefix rewinds *without* an anchor. Keeping one would leave the
/// next attempt a commit to `reset --hard` onto when the work is already on
/// the feature branch and nothing needs restoring.
#[test]
fn rewinding_a_merged_prefix_drops_the_anchor() {
    let resume = CheckpointResume::Merged {
        landed_ids: vec!["t-1".into(), "t-2".into()],
        produced: None,
    };
    let (ids, anchor, _) = resume.as_stored();
    assert_eq!(ids, ["t-1".to_string(), "t-2".to_string()]);
    assert_eq!(
        anchor, None,
        "a merged prefix needs no anchor; one would invite a pointless restore"
    );
}

/// A stranded prefix keeps both halves — this is the state a crash-resume
/// depends on, and a rollback in between must not spend it.
#[test]
fn rewinding_a_stranded_prefix_keeps_the_anchor() {
    let resume = CheckpointResume::Restore {
        landed_ids: vec!["t-1".into()],
        sha: ANCHOR.to_string(),
        produced: None,
    };
    let (ids, anchor, _) = resume.as_stored();
    assert_eq!(ids, ["t-1".to_string()]);
    assert_eq!(anchor, Some(ANCHOR));
}

/// The rewind has to survive being applied twice — a retry that rolls back
/// again reads its own rewound row. `Merged` re-reads as `Merged` (no
/// anchor), so the second rewind writes what the first one did.
#[test]
fn the_rewind_is_idempotent() {
    let first = CheckpointResume::Merged {
        landed_ids: vec!["t-1".into()],
        produced: Some(CheckpointProduced {
            artifact_refs: vec!["/store/f-1/s-impl/report.md".into()],
            satisfied_decls: vec!["report".into()],
        }),
    };
    let (ids, anchor, produced) = first.as_stored();
    let second = CheckpointResume::Merged {
        landed_ids: ids.to_vec(),
        produced: produced.cloned(),
    };
    assert_eq!(second.as_stored(), (ids, anchor, produced));
}
