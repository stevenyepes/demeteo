// Tests extracted from `crates/demeteo-core/src/domain/sync_session.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn probe(worktree_exists: bool, merge_in_progress: bool, dirty: bool) -> SyncWorkspaceProbe {
    SyncWorkspaceProbe {
        worktree_exists,
        merge_in_progress,
        dirty,
    }
}

/// The whole reason the table is not the answer: the tree a conflicted session
/// named is what the next sync force-removes, and after that there is nothing
/// left for the user to resolve, resume or abort.
#[test]
fn a_conflict_whose_worktree_is_gone_is_an_abandoned_sync() {
    let gone = probe(false, false, false);
    for stored in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::ResolutionFailed,
        SyncSessionStatus::Blocked,
        SyncSessionStatus::Syncing,
    ] {
        assert_eq!(
            reconcile(stored, Some(&gone)),
            SyncSessionStatus::Aborted,
            "{stored:?}"
        );
    }
}

/// A `resolving` row is only ever read by a process that did not write it, so
/// reading one means whoever was resolving is gone. The merge is still open,
/// which makes it a conflict waiting for somebody — not work in progress that
/// a caller should sit and wait on.
#[test]
fn a_resolve_nobody_is_driving_falls_back_to_the_conflict() {
    assert_eq!(
        reconcile(SyncSessionStatus::Resolving, Some(&probe(true, true, true))),
        SyncSessionStatus::Conflicted
    );
}

/// The agent, or the user in their own editor, committed the resolution. Git
/// says so — `MERGE_HEAD` is consumed and the tree is clean — and the row,
/// written before any of that happened, does not.
#[test]
fn a_closed_merge_over_a_clean_tree_is_resolved_whatever_the_row_says() {
    let done = probe(true, false, false);
    for stored in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::ResolutionFailed,
    ] {
        assert_eq!(
            reconcile(stored, Some(&done)),
            SyncSessionStatus::Resolved,
            "{stored:?}"
        );
    }
}

/// A push that failed leaves exactly the shape the previous test reads as
/// resolved — merge committed, tree clean — and nothing about it was ever
/// conflicted. Calling it "resolved" would offer a review of a resolution that
/// never happened.
#[test]
fn a_blocked_sync_over_a_clean_tree_stays_blocked() {
    assert_eq!(
        reconcile(SyncSessionStatus::Blocked, Some(&probe(true, false, false))),
        SyncSessionStatus::Blocked
    );
}

/// The row said resolved and git still holds an open merge, which happens when
/// a commit was attempted and refused — by a hook, or by an unmerged path the
/// staging check missed.
#[test]
fn a_resolution_the_merge_outlived_is_a_conflict_again() {
    assert_eq!(
        reconcile(SyncSessionStatus::Resolved, Some(&probe(true, true, false))),
        SyncSessionStatus::Conflicted
    );
}

/// `None` is "this session names no worktree", not "we looked and found
/// nothing". A sync that has not provisioned one yet would otherwise read as
/// abandoned on its very first poll.
#[test]
fn a_session_with_no_worktree_to_look_at_is_believed() {
    assert_eq!(
        reconcile(SyncSessionStatus::Syncing, None),
        SyncSessionStatus::Syncing
    );
    assert_eq!(
        reconcile(SyncSessionStatus::Blocked, None),
        SyncSessionStatus::Blocked
    );
}

/// Nothing on disk belongs to a finished sync, so nothing on disk may reopen
/// one — an aborted session whose directory a later step re-created must not
/// come back to life.
#[test]
fn a_terminal_status_survives_any_observation() {
    for stored in [
        SyncSessionStatus::UpToDate,
        SyncSessionStatus::Merged,
        SyncSessionStatus::Aborted,
    ] {
        assert!(stored.is_terminal(), "{stored:?}");
        assert_eq!(reconcile(stored, Some(&probe(true, true, true))), stored);
        assert_eq!(reconcile(stored, Some(&probe(false, false, false))), stored);
    }
}

/// The stored spelling is the wire spelling, and a row written by a build that
/// knows a status this one does not must not panic the reader.
#[test]
fn every_status_round_trips_through_its_stored_spelling() {
    for stored in [
        SyncSessionStatus::Syncing,
        SyncSessionStatus::UpToDate,
        SyncSessionStatus::Merged,
        SyncSessionStatus::Blocked,
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::Resolved,
        SyncSessionStatus::ResolutionFailed,
        SyncSessionStatus::Aborted,
    ] {
        assert_eq!(SyncSessionStatus::parse(stored.as_str()), Some(stored));
    }
    assert_eq!(SyncSessionStatus::parse("rebasing"), None);
}
