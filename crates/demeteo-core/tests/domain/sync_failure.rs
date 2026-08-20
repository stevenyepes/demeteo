// Tests extracted from `crates/demeteo-core/src/domain/sync_failure.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::models::ConflictReport;
use crate::ports::execution::{TIMEOUT_ERROR_PREFIX, TRANSPORT_ERROR_PREFIX};

const EVERY_STAGE: [SyncBlockedStage; 8] = [
    SyncBlockedStage::Fetch,
    SyncBlockedStage::BaseRefMissing,
    SyncBlockedStage::WorktreeProvision,
    SyncBlockedStage::Merge,
    SyncBlockedStage::Push,
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
