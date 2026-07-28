// Tests extracted from `crates/demeteo-core/src/domain/sequence/checkpoint.rs` (mirrored-tests convention). `super` = that module.

use super::*;

const ANCHOR: &str = "1111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222";

const EVERY_PROBE: [AnchorProbe; 4] = [
    AnchorProbe::Missing,
    AnchorProbe::Merged,
    AnchorProbe::Stranded,
    AnchorProbe::Unknown,
];

fn ids(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

/// No row at all: the step never checkpointed, or it completed and cleared.
fn empty_row() -> SequenceCheckpoint {
    SequenceCheckpoint {
        landed_task_ids: Vec::new(),
        anchor_sha: None,
        produced: None,
    }
}

/// A pre-V35 row: ids, no anchor. Only one writer existed then — the one
/// that merges the prefix before recording it.
fn anchorless_row() -> SequenceCheckpoint {
    SequenceCheckpoint {
        landed_task_ids: ids(&["t-1", "t-2"]),
        anchor_sha: None,
        produced: None,
    }
}

/// The current shape: ids plus the commit the prefix ends at.
fn anchored_row() -> SequenceCheckpoint {
    SequenceCheckpoint {
        landed_task_ids: ids(&["t-1", "t-2"]),
        anchor_sha: Some(ANCHOR.to_string()),
        produced: None,
    }
}

// ── The decision table, exhaustively ──────────────────────────────────────────
//
// Three row shapes × four probe verdicts. Every cell is asserted, because
// the cost of a wrong one is asymmetric and invisible: a wrong `None`
// re-runs tasks that were already paid for, while a wrong `Restore`
// `reset --hard`s a fresh worktree backwards over merged work.

/// An empty row means nothing landed, whatever git says about an anchor it
/// does not have.
#[test]
fn an_empty_row_never_resumes() {
    for probe in EVERY_PROBE {
        assert_eq!(
            classify(empty_row(), probe),
            CheckpointResume::None,
            "empty row + {probe:?}"
        );
    }
}

/// A pre-V35 row is `Merged` by construction — the only writer that
/// produced one merged first — so the probe is not consulted and cannot
/// change the answer.
#[test]
fn an_anchorless_row_is_merged_whatever_the_probe_says() {
    for probe in EVERY_PROBE {
        assert_eq!(
            classify(anchorless_row(), probe),
            CheckpointResume::Merged {
                landed_ids: ids(&["t-1", "t-2"]),
                produced: None,
            },
            "anchorless row + {probe:?}"
        );
    }
}

/// The anchor is contained in the feature branch: a freshly-cut worktree
/// already carries the prefix, so skip the ids and touch nothing.
#[test]
fn an_anchored_row_probed_merged_skips_without_restoring() {
    assert_eq!(
        classify(anchored_row(), AnchorProbe::Merged),
        CheckpointResume::Merged {
            landed_ids: ids(&["t-1", "t-2"]),
            produced: None,
        }
    );
}

/// The crash shape: the prefix is committed on the step branch and nowhere
/// else. This is the one verdict that moves a worktree.
#[test]
fn an_anchored_row_probed_stranded_restores_onto_the_anchor() {
    assert_eq!(
        classify(anchored_row(), AnchorProbe::Stranded),
        CheckpointResume::Restore {
            landed_ids: ids(&["t-1", "t-2"]),
            sha: Sha::new(ANCHOR),
            produced: None,
        }
    );
}

/// Uncertainty is not `Stranded`. A missing anchor has nothing to restore
/// onto, and an unanswerable probe knows nothing — both re-run the list,
/// which is the cheap mistake rather than the destructive one.
#[test]
fn an_anchored_row_resolves_every_uncertainty_to_a_full_rerun() {
    for probe in [AnchorProbe::Missing, AnchorProbe::Unknown] {
        assert_eq!(
            classify(anchored_row(), probe),
            CheckpointResume::None,
            "anchored row + {probe:?} must not skip or restore"
        );
    }
}

/// The payload rides along with whichever resume the row produces. `None`
/// means *unknown* (a pre-V36 row), never "produced nothing" — the
/// difference decides whether a step whose deliverable is already on disk
/// passes its declared-artifact check or is failed for one it did produce.
#[test]
fn the_produced_payload_survives_classification() {
    let produced = CheckpointProduced {
        artifact_refs: vec!["/store/f-1/s-impl/report.md".into()],
        satisfied_decls: vec!["report".into()],
    };
    let row = SequenceCheckpoint {
        landed_task_ids: ids(&["t-1"]),
        anchor_sha: Some(ANCHOR.to_string()),
        produced: Some(produced.clone()),
    };
    assert_eq!(
        classify(row.clone(), AnchorProbe::Merged).produced(),
        Some(&produced)
    );
    assert_eq!(
        classify(row, AnchorProbe::Stranded).produced(),
        Some(&produced)
    );
    assert_eq!(
        classify(anchored_row(), AnchorProbe::Merged).produced(),
        None,
        "a pre-V36 row cannot say, and must not be read as having said 'nothing'"
    );
}

// ── anchor_is_merged ──────────────────────────────────────────────────────────

/// `git merge-base <anchor> <base>` printing the anchor itself is the only
/// evidence that the prefix already reached the feature branch.
#[test]
fn the_merge_base_being_the_anchor_means_merged() {
    assert!(anchor_is_merged(&format!("{ANCHOR}\n"), &Sha::new(ANCHOR)));
}

/// An *earlier* common ancestor means the anchor is off on the step branch:
/// the crash shape, and the only case that may reset a worktree.
#[test]
fn an_earlier_merge_base_means_the_prefix_is_stranded() {
    assert!(!anchor_is_merged(&format!("{OTHER}\n"), &Sha::new(ANCHOR)));
}

/// The failure this probe shape exists to prevent. `merge-base` answers on
/// stdout, so "git could not answer" arrives as an `Err` the caller turns
/// into a full re-run — never as output that reads like a verdict. Empty or
/// unparseable output must not be mistaken for a match either.
#[test]
fn no_answer_is_never_read_as_merged() {
    let anchor = Sha::new(ANCHOR);
    assert!(!anchor_is_merged("", &anchor));
    assert!(!anchor_is_merged("   \n", &anchor));
    assert!(!anchor_is_merged("fatal: not a git repository\n", &anchor));
}

/// Git prints lowercase hex; a checkpoint written from a differently-cased
/// source must still match rather than silently resolving to "stranded" and
/// resetting a worktree backwards over merged work.
#[test]
fn the_comparison_ignores_sha_case_and_whitespace() {
    assert!(anchor_is_merged(
        "  1111111111111111111111111111111111111111  \n",
        &Sha::new("1111111111111111111111111111111111111111")
    ));
    assert!(anchor_is_merged(
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\n",
        &Sha::new("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    ));
}

// ── as_stored: what a rollback writes back ────────────────────────────────────

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
        sha: Sha::new(ANCHOR),
        produced: None,
    };
    let (ids, anchor, _) = resume.as_stored();
    assert_eq!(ids, ["t-1".to_string()]);
    assert_eq!(anchor, Some(&Sha::new(ANCHOR)));
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

/// What a rewind writes back must read back as the same resume, or a
/// rollback would quietly change what the next attempt does. This closes
/// the loop between the two halves of the module: `as_stored` reconstructs
/// a row, and `classify` reads that row.
#[test]
fn a_rewound_merged_row_reclassifies_as_merged() {
    let original = CheckpointResume::Merged {
        landed_ids: ids(&["t-1", "t-2"]),
        produced: None,
    };
    let (landed, anchor, produced) = original.as_stored();
    let rewritten = SequenceCheckpoint {
        landed_task_ids: landed.to_vec(),
        anchor_sha: anchor.map(|s| s.as_str().to_string()),
        produced: produced.cloned(),
    };
    // No anchor was written, so no probe is consulted — which is exactly
    // why the rewind drops it.
    assert_eq!(classify(rewritten, AnchorProbe::Unknown), original);
}
