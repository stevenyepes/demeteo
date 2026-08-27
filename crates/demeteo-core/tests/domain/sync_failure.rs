// Tests extracted from `crates/demeteo-core/src/domain/sync_failure.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::models::ConflictReport;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};

const EVERY_STAGE: [SyncBlockedStage; 10] = [
    SyncBlockedStage::Fetch,
    SyncBlockedStage::BaseRefMissing,
    SyncBlockedStage::WorktreeProvision,
    SyncBlockedStage::Merge,
    SyncBlockedStage::Push,
    SyncBlockedStage::Verify,
    SyncBlockedStage::FeatureDiverged,
    SyncBlockedStage::RepoContext,
    SyncBlockedStage::HeldResolution,
    SyncBlockedStage::TurnInFlight,
];

fn blocked(stage: SyncBlockedStage) -> UpstreamSyncFailure {
    UpstreamSyncFailure::Blocked {
        stage,
        raw_error: "git: could not read from remote repository".to_string(),
    }
}

fn conflict(files: Vec<ConflictFile>) -> UpstreamSyncFailure {
    UpstreamSyncFailure::Conflict {
        report: ConflictReport {
            source_branch: "origin/main".to_string(),
            target_branch: "demeteo/features/f-1".to_string(),
            files,
            raw_error: "CONFLICT (content): Merge conflict in README.md".to_string(),
            detected_at: 0,
            worktree_path: Some("/w/sync-f-1".to_string()),
        },
        worktree_path: Some("/w/sync-f-1".to_string()),
        resolves_the_base_merge: true,
    }
}

/// Every stage that stops short of a merge renders as the same class, because
/// the user's move for all of them is "fix the thing git named", never "spend
/// an agent on it".
#[test]
fn no_stage_that_never_merged_renders_as_a_conflict() {
    for stage in EVERY_STAGE {
        match view_for(Err(blocked(stage))) {
            SyncOutcomeView::Blocked {
                stage: seen,
                raw_error,
            } => {
                assert_eq!(seen, stage);
                assert!(raw_error.contains("remote repository"), "{raw_error}");
            }
            other => panic!("{stage:?} rendered as {other:?}"),
        }
    }
}

/// The heuristic this replaced: an empty file list read as "not really a
/// conflict". `parse_unmerged` answers an empty vec on any transport error, so
/// a real conflict whose porcelain read failed has the same payload as a fetch
/// that never ran — and only the carried class tells them apart.
#[test]
fn a_conflict_with_no_parsed_files_is_still_a_conflict() {
    match view_for(Err(conflict(Vec::new()))) {
        SyncOutcomeView::Conflict {
            conflict_files,
            raw_error,
        } => {
            assert!(conflict_files.is_empty());
            assert!(raw_error.contains("Merge conflict"), "{raw_error}");
        }
        other => panic!("an unparsed conflict rendered as {other:?}"),
    }
}

#[test]
fn a_merge_that_left_unmerged_paths_renders_as_a_conflict() {
    let files = vec![ConflictFile {
        path: "README.md".to_string(),
        kind: "both-modified".to_string(),
    }];
    match view_for(Err(conflict(files))) {
        SyncOutcomeView::Conflict { conflict_files, .. } => {
            assert_eq!(conflict_files.len(), 1);
            assert_eq!(conflict_files[0].path, "README.md");
        }
        other => panic!("a conflicted merge rendered as {other:?}"),
    }
}

/// The resolution turn opens by probing for `MERGE_HEAD` and reports its
/// absence as "run 'Sync with main' first" — so routing a blocked failure into
/// it replaces the real cause with an instruction to redo what just failed.
#[test]
fn a_blocked_sync_never_reaches_the_resolver() {
    for stage in EVERY_STAGE {
        let failure = blocked(stage);
        match step_next(&failure) {
            SyncStepNext::Fail(raw_error) => {
                assert!(raw_error.contains("remote repository"), "{raw_error}")
            }
            SyncStepNext::Resolve { .. } => panic!("{stage:?} was routed to the resolution agent"),
        }
    }
}

#[test]
fn a_conflict_carries_its_worktree_to_the_resolver() {
    let failure = conflict(vec![ConflictFile {
        path: "README.md".to_string(),
        kind: "both-modified".to_string(),
    }]);
    assert_eq!(
        step_next(&failure),
        SyncStepNext::Resolve {
            files: &[ConflictFile {
                path: "README.md".to_string(),
                kind: "both-modified".to_string(),
            }],
            worktree_path: Some("/w/sync-f-1"),
            resolves_the_base_merge: true,
        }
    );
}

/// `src/types.ts` declares these literals to receive them, and there is no
/// `Deserialize` on the view to round-trip through, so the spelling is pinned
/// against the serializer directly. Pinning a sample of the variants left the
/// rest free to drift, and a stage the TS union does not carry arrives silent:
/// the banner's per-stage sentence resolves to `undefined`, which React renders
/// as nothing at all — no next move, no console error. The `match` below is
/// what makes that impossible to add by accident.
#[test]
fn the_serialized_shape_is_the_wire_contract() {
    let json = |v: &SyncOutcomeView| serde_json::to_string(v).expect("SyncOutcomeView serializes");

    for stage in EVERY_STAGE {
        let wire = match stage {
            SyncBlockedStage::Fetch => "fetch",
            SyncBlockedStage::BaseRefMissing => "base_ref_missing",
            SyncBlockedStage::WorktreeProvision => "worktree_provision",
            SyncBlockedStage::Merge => "merge",
            SyncBlockedStage::Push => "push",
            SyncBlockedStage::Verify => "verify",
            SyncBlockedStage::FeatureDiverged => "feature_diverged",
            SyncBlockedStage::RepoContext => "repo_context",
            SyncBlockedStage::HeldResolution => "held_resolution",
            SyncBlockedStage::TurnInFlight => "turn_in_flight",
        };
        assert_eq!(
            json(&SyncOutcomeView::Blocked {
                stage,
                raw_error: "nope".to_string(),
            }),
            format!(r#"{{"status":"blocked","stage":"{wire}","raw_error":"nope"}}"#)
        );
        assert_eq!(
            stage.as_str(),
            wire,
            "the column and the payload are read back into the one TS union, so a \
             stage cannot be spelled one way by serde and another by the column"
        );
    }

    assert_eq!(
        json(&SyncOutcomeView::Resolved {
            merge_commit_sha: "abc1234".to_string(),
        }),
        r#"{"status":"resolved","merge_commit_sha":"abc1234"}"#
    );
}

/// The merge is the one call whose failure can be either class, and the
/// payload cannot tell them apart: a dropped channel and a conflicted tree
/// both come back as an `Err` string with an empty file list behind it.
#[test]
fn a_merge_that_never_answered_is_not_a_conflict() {
    assert_eq!(
        merge_failure_stage(&format!("{TRANSPORT_ERROR_PREFIX}Connection appears dead")),
        Some(SyncBlockedStage::Merge)
    );
    assert_eq!(
        merge_failure_stage(&format!("{TIMEOUT_ERROR_PREFIX}exceeded 600s")),
        Some(SyncBlockedStage::Merge)
    );
    assert_eq!(
        merge_failure_stage("CONFLICT (content): Merge conflict in README.md"),
        None,
        "a non-zero exit is the only shape that reached a verdict"
    );
}

/// The other direction of the same spelling. `parse` answers `None` for
/// anything it does not know, which is what makes a row from a newer build
/// harmless — and what makes a stage this build *writes* and cannot read back
/// indistinguishable from one, all the way to the pane's unnamed-block copy.
#[test]
fn every_stage_round_trips_through_the_column() {
    for stage in EVERY_STAGE {
        assert_eq!(
            SyncBlockedStage::parse(stage.as_str()),
            Some(stage),
            "{stage:?} is written as `{}` and read back as nothing",
            stage.as_str()
        );
    }
}

/// The reconcile runs before the base merge, so its conflict is one the sync
/// has to be pressed through twice — and the flag is what tells the unattended
/// node that a resolution of it is not a sync.
#[test]
fn a_reconcile_conflict_does_not_report_the_base_as_merged() {
    let files = vec![ConflictFile {
        path: "README.md".to_string(),
        kind: "both-modified".to_string(),
    }];
    let mut failure = conflict(files);
    if let UpstreamSyncFailure::Conflict {
        resolves_the_base_merge,
        ..
    } = &mut failure
    {
        *resolves_the_base_merge = false;
    }
    match step_next(&failure) {
        SyncStepNext::Resolve {
            resolves_the_base_merge,
            ..
        } => assert!(!resolves_the_base_merge),
        other => panic!("a conflict routed to {other:?}"),
    }
}

/// A base merge that did not land after a resolved reconcile leaves the one
/// state no other verdict describes, and the node's words for it have to name
/// both halves — the branch was reconciled and it was not synced.
#[test]
fn the_base_merge_after_a_reconcile_says_which_half_landed() {
    let reason = base_merge_refusal(
        "main",
        &conflict(vec![ConflictFile {
            path: "README.md".to_string(),
            kind: "both-modified".to_string(),
        }]),
    );
    assert!(reason.contains("'main' was not merged"), "{reason}");
    assert!(reason.contains("README.md"), "{reason}");

    assert_eq!(
        base_merge_refusal("main", &blocked(SyncBlockedStage::Fetch)),
        "git: could not read from remote repository",
        "a block already carries git's own words"
    );
}

/// A press answered with the previous sync's row is a press that reads as
/// having done something. Every stage returned before `sync_sessions.open()`
/// has to say so, and the held-resolution refusal is returned before even the
/// turn slot is claimed.
#[test]
fn every_refusal_raised_before_the_row_exists_says_so() {
    for stage in [
        SyncBlockedStage::RepoContext,
        SyncBlockedStage::TurnInFlight,
        SyncBlockedStage::HeldResolution,
    ] {
        assert!(
            stage.precedes_the_session(),
            "{stage:?} is raised before the session row is opened"
        );
    }
    for stage in EVERY_STAGE.into_iter().filter(|s| {
        !matches!(
            s,
            SyncBlockedStage::RepoContext
                | SyncBlockedStage::TurnInFlight
                | SyncBlockedStage::HeldResolution
        )
    }) {
        assert!(
            !stage.precedes_the_session(),
            "{stage:?} has a row of its own and must be read off it"
        );
    }
}

/// A reset has no second side, so nothing it leaves is unmerged: `None` here
/// is the answer that sends a resolver into a tree with no `MERGE_HEAD`.
#[test]
fn a_reset_that_git_refused_is_never_a_conflict() {
    use crate::domain::upstream_feature::DivergenceReconcile;

    let unmerged = "error: Entry 'README.md' not uptodate. Cannot merge.";
    assert_eq!(
        reconcile_failure_stage(DivergenceReconcile::ResetOntoOrigin, unmerged),
        Some(SyncBlockedStage::FeatureDiverged)
    );
    assert_eq!(
        reconcile_failure_stage(DivergenceReconcile::MergeOrigin, unmerged),
        None,
        "a merge that left unmerged paths is the resolver's"
    );

    for prefix in [TRANSPORT_ERROR_PREFIX, TIMEOUT_ERROR_PREFIX] {
        let cut_short = format!("{prefix}connection closed");
        for move_ in [
            DivergenceReconcile::ResetOntoOrigin,
            DivergenceReconcile::MergeOrigin,
        ] {
            assert_eq!(
                reconcile_failure_stage(move_, &cut_short),
                Some(SyncBlockedStage::Merge),
                "{move_:?} over {prefix:?} never reached a verdict"
            );
        }
    }
}
