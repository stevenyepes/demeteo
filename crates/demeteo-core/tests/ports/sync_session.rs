// Tests extracted from `crates/demeteo-core/src/ports/sync_session.rs` (mirrored-tests convention). `super` = that module.

use super::*;

fn succeeded(published: bool, worktree_discarded: bool) -> SyncResolution {
    SyncResolution::Succeeded {
        merge_commit_sha: "c0ffeec".to_string(),
        published,
        worktree_discarded,
    }
}

/// The column the review card is gated on, written from the outcome in both
/// directions. A `Succeeded` that did not publish must *clear* it rather than
/// leave it alone: a patch that says nothing about publication makes the row's
/// meaning depend on how it got there, and the one state P4 exists to create is
/// the one that reads as finished when it does.
#[test]
fn a_resolution_that_did_not_publish_says_so_rather_than_staying_quiet() {
    let held = SyncSessionPatch::from_resolution(&succeeded(false, false), 5);
    assert_eq!(held.pushed_at, Some(None));
    assert_eq!(held.status, Some(SyncSessionStatus::Resolved));

    let sent = SyncSessionPatch::from_resolution(&succeeded(true, true), 5);
    assert_eq!(sent.pushed_at, Some(Some(5)));
}

/// The tree may only stop being named once somebody watched it go. A held
/// resolution keeps its worktree — it is where `discard_resolution` puts the
/// branch back — and a teardown nobody confirmed is the same as no teardown.
#[test]
fn only_an_observed_teardown_lets_the_row_stop_naming_the_worktree() {
    assert_eq!(
        SyncSessionPatch::from_resolution(&succeeded(true, true), 0).worktree_path,
        Some(None)
    );
    for unconfirmed in [succeeded(true, false), succeeded(false, false)] {
        assert_eq!(
            SyncSessionPatch::from_resolution(&unconfirmed, 0).worktree_path,
            None,
            "{unconfirmed:?}"
        );
    }
}

/// A turn that started or failed says nothing about publication or the tree:
/// the row is mid-sync, and every column it does not own has to be left for
/// whoever does.
#[test]
fn a_turn_that_has_not_landed_writes_only_what_it_knows() {
    let started = SyncSessionPatch::from_resolution(&SyncResolution::Started, 7);
    assert_eq!(started.status, Some(SyncSessionStatus::Resolving));
    assert_eq!(started.pushed_at, None);
    assert_eq!(started.merge_commit_sha, None);
    assert_eq!(started.raw_error, None);

    let failed = SyncSessionPatch::from_resolution(
        &SyncResolution::Failed {
            reason: "markers left behind".to_string(),
        },
        7,
    );
    assert_eq!(failed.status, Some(SyncSessionStatus::ResolutionFailed));
    assert_eq!(failed.pushed_at, None);
    assert_eq!(failed.worktree_path, None);
    assert_eq!(
        failed.raw_error,
        Some(Some("markers left behind".to_string()))
    );
}

/// A success is the one outcome that contradicts a stored failure, so it is the
/// one that has to clear it.
///
/// The resolver works in rounds, and a round that trips its turn cap writes its
/// reason here before the next one runs. Left standing, the row that finally
/// resolved reads back as `resolved` beside "the agent stopped at its turn cap"
/// — a verdict and its own refutation, with nothing to say which happened last.
#[test]
fn a_landed_resolution_clears_the_reason_an_earlier_round_left() {
    assert_eq!(
        SyncSessionPatch::from_resolution(&succeeded(false, false), 5).raw_error,
        Some(None)
    );
    assert_eq!(
        SyncSessionPatch::from_resolution(&succeeded(true, true), 5).raw_error,
        Some(None)
    );
}

/// The row counts a resolution when it starts, and only then.
///
/// The column, the port field and the Sync pane's own `Attempts` metric all
/// existed; nothing ever set the field. A user who pressed "Resolve with agent"
/// three times read "Attempts 0" all three times, with no way to tell a turn
/// that never ran from three that did. Counted at the start rather than the
/// verdict so a resolution still running already shows as an attempt.
#[test]
fn a_resolution_is_counted_when_it_starts_and_not_again_when_it_ends() {
    assert!(
        SyncSessionPatch::from_resolution(&SyncResolution::Started, 7).bump_attempts,
        "the turn beginning is the attempt"
    );
    assert!(
        !SyncSessionPatch::from_resolution(
            &SyncResolution::Failed {
                reason: "markers left behind".to_string(),
            },
            7,
        )
        .bump_attempts,
        "its verdict is the same attempt, not a second one"
    );
    assert!(
        !SyncSessionPatch::from_resolution(
            &SyncResolution::Succeeded {
                merge_commit_sha: "c0ffee".to_string(),
                published: true,
                worktree_discarded: true,
            },
            7,
        )
        .bump_attempts
    );
}
