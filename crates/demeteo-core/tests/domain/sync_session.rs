// Tests extracted from `crates/demeteo-core/src/domain/sync_session.rs` (mirrored-tests convention). `super` = that module.

use super::*;
use crate::domain::sync_failure::SyncBlockedStage;

fn probe(worktree_exists: bool, merge_in_progress: bool, dirty: bool) -> SyncWorkspaceProbe {
    SyncWorkspaceProbe {
        worktree_exists,
        merge_in_progress,
        dirty,
        head_advanced: None,
    }
}

/// The corrected status, with nothing running.
///
/// That is the standing every correction below is about: a row whose writer is
/// gone is the only one a probe is entitled to move, and the two statuses a
/// live writer still holds have tests of their own.
fn dead(stored: SyncSessionStatus, probe: Option<&SyncWorkspaceProbe>) -> SyncSessionStatus {
    reconcile(stored, probe, SyncLiveness::Gone).status
}

fn probe_head(
    worktree_exists: bool,
    merge_in_progress: bool,
    dirty: bool,
    head_advanced: bool,
) -> SyncWorkspaceProbe {
    SyncWorkspaceProbe {
        head_advanced: Some(head_advanced),
        ..probe(worktree_exists, merge_in_progress, dirty)
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
            dead(stored, Some(&gone)),
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
        dead(SyncSessionStatus::Resolving, Some(&probe(true, true, true))),
        SyncSessionStatus::Conflicted
    );
}

/// The agent, or the user in their own editor, committed the resolution. Git
/// says so — `MERGE_HEAD` is consumed, the tree is clean, and `HEAD` has moved
/// off the sha the sync started from — and the row, written before any of that
/// happened, does not.
#[test]
fn a_closed_merge_over_a_clean_tree_is_resolved_whatever_the_row_says() {
    let done = probe_head(true, false, false, true);
    for stored in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::ResolutionFailed,
    ] {
        assert_eq!(
            dead(stored, Some(&done)),
            SyncSessionStatus::Resolved,
            "{stored:?}"
        );
    }
}

/// `git merge --abort` in the user's own terminal leaves a tree that is
/// byte-identical to a committed resolution — merge closed, nothing modified —
/// and the two want opposite answers. `HEAD` back on the sha the sync started
/// from is the whole of the difference; calling this `resolved` would offer a
/// review of a merge that no longer exists, and P4 would push a branch carrying
/// none of it.
#[test]
fn a_merge_undone_by_hand_is_abandoned_not_resolved() {
    let undone = probe_head(true, false, false, false);
    for stored in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::ResolutionFailed,
    ] {
        assert_eq!(
            dead(stored, Some(&undone)),
            SyncSessionStatus::Aborted,
            "{stored:?}"
        );
    }
}

/// With no starting sha on the row there is nothing to compare `HEAD` against,
/// so neither verdict above is earned and the claim already on the row stands.
/// Guessing either way is how a resolution gets reviewed that never happened.
#[test]
fn a_closed_merge_with_no_starting_sha_leaves_the_row_alone() {
    let unknowable = probe(true, false, false);
    for stored in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::ResolutionFailed,
    ] {
        assert_eq!(dead(stored, Some(&unknowable)), stored, "{stored:?}");
    }
}

/// A push that failed leaves exactly the shape the previous test reads as
/// resolved — merge committed, tree clean — and nothing about it was ever
/// conflicted. Calling it "resolved" would offer a review of a resolution that
/// never happened.
#[test]
fn a_blocked_sync_over_a_clean_tree_stays_blocked() {
    assert_eq!(
        dead(SyncSessionStatus::Blocked, Some(&probe(true, false, false))),
        SyncSessionStatus::Blocked
    );
}

/// The row said resolved and git still holds an open merge, which happens when
/// a commit was attempted and refused — by a hook, or by an unmerged path the
/// staging check missed.
#[test]
fn a_resolution_the_merge_outlived_is_a_conflict_again() {
    assert_eq!(
        dead(SyncSessionStatus::Resolved, Some(&probe(true, true, false))),
        SyncSessionStatus::Conflicted
    );
}

/// `None` is "this session names no worktree", not "we looked and found
/// nothing", so nothing about the tree may move the row.
///
/// `Syncing` is the one status that is corrected without a tree, and on the
/// other observation entirely — see
/// [`a_merge_nobody_is_running_never_reached_a_verdict`].
#[test]
fn a_session_with_no_worktree_to_look_at_is_believed() {
    assert_eq!(
        dead(SyncSessionStatus::Blocked, None),
        SyncSessionStatus::Blocked
    );
    assert_eq!(
        dead(SyncSessionStatus::Conflicted, None),
        SyncSessionStatus::Conflicted
    );
    assert_eq!(
        reconcile(SyncSessionStatus::Syncing, None, SyncLiveness::Live).status,
        SyncSessionStatus::Syncing,
        "a merge that has not provisioned its tree yet is still a merge"
    );
}

/// The safety bug this observation exists for.
///
/// `resolving` was corrected to `conflicted` on the strength of an open merge
/// alone, on the reasoning that only a process other than the writer ever reads
/// one. Both resolution entry points now write it *while their turn runs*, and
/// a session corrected out from under a live turn is offered Abort — which
/// issues `git merge --abort`, `worktree remove --force` and a recursive delete
/// on the directory the agent is editing in.
#[test]
fn a_live_resolution_is_not_corrected_out_from_under_itself() {
    let open_merge = probe(true, true, false);
    assert_eq!(
        reconcile(
            SyncSessionStatus::Resolving,
            Some(&open_merge),
            SyncLiveness::Live
        )
        .status,
        SyncSessionStatus::Resolving
    );
    assert!(
        !user_may_intervene(unpublished(SyncSessionStatus::Resolving, "completed")),
        "and nothing destructive is offered for it"
    );
    assert_eq!(
        dead(SyncSessionStatus::Resolving, Some(&open_merge)),
        SyncSessionStatus::Conflicted,
        "a resolver that really did die still owes the conflict back"
    );
    assert!(user_may_intervene(unpublished(
        SyncSessionStatus::Conflicted,
        "completed"
    )));
}

/// What survives a restart, and what must not.
///
/// The process-local claim is gone — that is the point of it being process
/// local — so a turn that died with its process answers `Gone` and its session
/// becomes recoverable again. A run the feature's own status still vouches for
/// answers `Live` on the same terms every other refusal already reads it on,
/// and run recovery is what retires that, not this.
#[test]
fn a_claim_does_not_outlive_the_process_that_made_it() {
    assert_eq!(sync_liveness(true, "completed"), SyncLiveness::Live);
    assert_eq!(
        sync_liveness(false, "completed"),
        SyncLiveness::Gone,
        "an empty registry after a restart is what gives the conflict back"
    );
    assert_eq!(
        sync_liveness(false, "running"),
        SyncLiveness::Live,
        "a driver still owns its own sync with no claim in any map"
    );
}

/// A sync cut short mid-merge, which used to be the one state nothing could
/// correct and no user action could clear: `syncing` passed through every
/// reconcile and every intervention was refused for it, so only the next sync's
/// force-remove ever reclaimed the worktree.
///
/// `Blocked` and not `Conflicted`: nothing established that the tree holds
/// unmerged paths, and reading it as a conflict is what sends the resolver
/// looking for a `MERGE_HEAD` nobody wrote. Blocked offers the abort that
/// reclaims the directory and withholds the resolver.
#[test]
fn a_merge_nobody_is_running_never_reached_a_verdict() {
    for observation in [
        None,
        Some(probe(true, true, false)),
        Some(probe(true, false, true)),
    ] {
        let verdict = reconcile(
            SyncSessionStatus::Syncing,
            observation.as_ref(),
            SyncLiveness::Gone,
        );
        assert_eq!(
            verdict.status,
            SyncSessionStatus::Blocked,
            "{observation:?}"
        );
        assert_eq!(
            verdict.blocked_stage,
            Some(SyncBlockedStage::Merge),
            "{observation:?}"
        );
    }
    assert!(
        user_may_intervene(unpublished(SyncSessionStatus::Blocked, "completed")),
        "and the user can finally clear it"
    );
    assert_eq!(
        reconcile(
            SyncSessionStatus::Syncing,
            Some(&probe(true, true, false)),
            SyncLiveness::Live
        ),
        SyncSessionStatus::Syncing.into(),
        "a merge that is still running is not a merge that stopped"
    );
}

/// The one blocked stage that leaves work behind, told from the six that do
/// not.
///
/// A `push` failure has already committed the merge onto the feature branch.
/// Offered only a retry, the second sync finds `origin/<base>` already merged,
/// changes nothing, reports up to date — and the unpublished merge stays in a
/// worktree the next sync force-removes. Publishing is the press that finishes
/// it, and it is reachable only because the stage is on the row (V46).
#[test]
fn only_a_push_blocked_sync_has_a_merge_to_publish() {
    use crate::domain::sync_session::{intervention_refusal, published_status, SyncIntervention};

    let blocked = |stage, published| SyncStanding {
        status: SyncSessionStatus::Blocked,
        published,
        blocked_stage: stage,
        feature_status: "completed",
        liveness: SyncLiveness::Gone,
    };
    assert_eq!(
        intervention_refusal(
            SyncIntervention::Publish,
            blocked(Some(SyncBlockedStage::Push), false)
        ),
        None
    );
    for stage in [
        None,
        Some(SyncBlockedStage::Fetch),
        Some(SyncBlockedStage::BaseRefMissing),
        Some(SyncBlockedStage::WorktreeProvision),
        Some(SyncBlockedStage::Merge),
        Some(SyncBlockedStage::RepoContext),
        Some(SyncBlockedStage::HeldResolution),
    ] {
        assert!(
            intervention_refusal(SyncIntervention::Publish, blocked(stage, false)).is_some(),
            "{stage:?} merged nothing, so there is nothing of it to publish"
        );
    }
    assert!(
        intervention_refusal(
            SyncIntervention::Publish,
            blocked(Some(SyncBlockedStage::Push), true)
        )
        .is_some(),
        "pressing it twice is not a second push"
    );
    assert_eq!(
        published_status(SyncSessionStatus::Blocked),
        SyncSessionStatus::Merged,
        "the failed push was the whole of the block"
    );
    assert_eq!(
        published_status(SyncSessionStatus::Resolved),
        SyncSessionStatus::Resolved,
        "a resolution's publication is `pushed_at`, and the review card reads the status"
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
        assert_eq!(dead(stored, Some(&probe(true, true, true))), stored);
        assert_eq!(dead(stored, Some(&probe(false, false, false))), stored);
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

/// A session in a given standing, with nothing published of it — what every
/// state below one is, and the shape the refusals are judged against.
fn unpublished(status: SyncSessionStatus, feature_status: &str) -> SyncStanding<'_> {
    SyncStanding {
        status,
        published: false,
        blocked_stage: None,
        feature_status,
        liveness: SyncLiveness::Gone,
    }
}

/// A conflict the user may act on, and one they may not. The destructive half
/// of the banner — abort deletes the worktree, resolve spawns a second agent in
/// it — must be unreachable while something else holds the tree, and a session
/// that says `resolving` says exactly that.
#[test]
fn a_sync_somebody_else_is_driving_is_not_the_users_to_touch() {
    for status in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::ResolutionFailed,
        SyncSessionStatus::Blocked,
    ] {
        assert!(
            user_may_intervene(unpublished(status, "completed")),
            "{status:?}"
        );
    }
    assert!(
        !user_may_intervene(unpublished(SyncSessionStatus::Resolving, "completed")),
        "a turn is holding this worktree"
    );
    // A resolution nobody has published is the user's — that is the whole of
    // the review state. Published, it is finished and there is nothing to
    // offer.
    assert!(user_may_intervene(unpublished(
        SyncSessionStatus::Resolved,
        "completed"
    )));
    assert!(!user_may_intervene(SyncStanding {
        status: SyncSessionStatus::Resolved,
        published: true,
        blocked_stage: None,
        feature_status: "completed",
        liveness: SyncLiveness::Gone,
    }));
    for status in [
        SyncSessionStatus::Syncing,
        SyncSessionStatus::UpToDate,
        SyncSessionStatus::Merged,
        SyncSessionStatus::Aborted,
    ] {
        assert!(
            !user_may_intervene(unpublished(status, "completed")),
            "{status:?}"
        );
    }
}

/// The window the session status cannot cover on its own: between the merge
/// failing and the resolution turn recording itself the row honestly reads
/// `conflicted`, and the step is still the one holding it. A live run is
/// therefore disqualifying regardless of what the session says.
#[test]
fn a_live_run_owns_its_sync_whatever_the_session_says() {
    for feature_status in [
        "pending",
        "running",
        "verifying",
        "awaiting_gate",
        "gated",
        "syncing_origin",
    ] {
        assert!(
            !user_may_intervene(unpublished(SyncSessionStatus::Conflicted, feature_status)),
            "{feature_status}"
        );
    }
    for feature_status in ["completed", "failed", "cancelled", "awaiting_mr"] {
        assert!(
            user_may_intervene(unpublished(SyncSessionStatus::Conflicted, feature_status)),
            "{feature_status}"
        );
    }
}

/// Abort and resolve do not accept the same sessions, and one predicate for
/// both is what let the resolve IPC reach a `Blocked` row: the turn then fails
/// its own preflight and files `resolution_failed`, replacing the
/// `UpstreamSyncFailure` text that row exists to keep with a verdict about a
/// merge that never happened.
#[test]
fn a_blocked_sync_may_be_abandoned_but_not_resolved() {
    use crate::domain::sync_session::{intervention_refusal, SyncIntervention};

    assert_eq!(
        intervention_refusal(
            SyncIntervention::Abort,
            unpublished(SyncSessionStatus::Blocked, "completed")
        ),
        None,
        "a blocked sync still holds an unpublished merge to undo"
    );
    let refusal = intervention_refusal(
        SyncIntervention::Resolve,
        unpublished(SyncSessionStatus::Blocked, "completed"),
    )
    .expect("there are no conflicts in a sync that never merged");
    assert!(refusal.contains("before it reached a merge"), "{refusal}");

    for action in [SyncIntervention::Abort, SyncIntervention::Resolve] {
        assert_eq!(
            intervention_refusal(
                action,
                unpublished(SyncSessionStatus::Conflicted, "completed")
            ),
            None,
            "{action:?}"
        );
        assert!(
            intervention_refusal(
                action,
                unpublished(SyncSessionStatus::Resolving, "completed")
            )
            .is_some(),
            "{action:?}"
        );
        assert!(
            intervention_refusal(
                action,
                unpublished(SyncSessionStatus::Conflicted, "running")
            )
            .is_some(),
            "{action:?}"
        );
    }
}

/// The state P4 introduced, and the one place it could be lied about.
///
/// A resolution's commit is on the feature branch; the sync worktree it was
/// made in is a throwaway that publishing deletes and that any later sync
/// force-removes. Folding "the tree is gone" into `Aborted` for this status
/// would tell the reader the merge was abandoned while it is sitting on their
/// branch waiting to be published — and `Aborted` is terminal, so nothing
/// would ever revisit it.
#[test]
fn a_resolution_survives_the_worktree_it_was_made_in() {
    assert_eq!(
        dead(
            SyncSessionStatus::Resolved,
            Some(&probe(false, false, false))
        ),
        SyncSessionStatus::Resolved
    );
}

/// The setting may take review away and may not impose it.
///
/// Nothing offers Publish or Discard while a driver holds the feature, so a
/// resolution held there would wait for a press that no surface can produce:
/// the run finishes, the branch never gets the merge, and the only evidence is
/// a row nobody looks at. Holding is therefore conditional on somebody being
/// able to act, and the project's own `true` cannot override that.
#[test]
fn a_resolution_only_waits_when_somebody_can_look_at_it() {
    use crate::domain::sync_session::{publish_policy, ResolutionPublish};

    for setting in [None, Some(true)] {
        assert_eq!(
            publish_policy(setting, true),
            ResolutionPublish::HoldForReview,
            "{setting:?}"
        );
        assert_eq!(
            publish_policy(setting, false),
            ResolutionPublish::Push,
            "{setting:?} with nobody watching"
        );
    }
    for reviewable in [true, false] {
        assert_eq!(
            publish_policy(Some(false), reviewable),
            ResolutionPublish::Push,
            "opted out, reviewable={reviewable}"
        );
    }
}

/// Which runs are "unattended" is not a second notion of attendedness: it is
/// the same [`run_is_live`] set every refusal above is built on. A workflow's
/// `sync` node only ever runs while its driver holds the feature, so a headless
/// run and a detached one both answer here without anything having to ask which
/// binary or which transport is executing.
#[test]
fn a_run_that_still_owns_its_branch_has_nobody_to_review_for() {
    use crate::domain::sync_session::resolution_is_reviewable;

    for feature_status in [
        "pending",
        "running",
        "verifying",
        "awaiting_gate",
        "gated",
        "syncing_origin",
    ] {
        assert!(
            !resolution_is_reviewable(feature_status),
            "{feature_status}"
        );
    }
    for feature_status in ["completed", "failed", "cancelled", "awaiting_mr"] {
        assert!(resolution_is_reviewable(feature_status), "{feature_status}");
    }
}

/// The row is one per feature and `open` is an upsert, so the next sync writes
/// over whatever the last one left. That is right for every session but the one
/// nobody has read: `head_before` is unrecoverable, the merge becomes part of
/// the new baseline on its way to origin, and — because the second merge finds
/// `origin/<base>` already in the branch and so changes nothing — the row lands
/// on a terminal `up_to_date` that every intervention then refuses. The
/// affordance that publishes the merge disappears with it.
#[test]
fn a_resolution_nobody_has_read_is_not_something_the_next_sync_may_write_over() {
    use crate::domain::sync_session::resync_refusal;

    assert!(resync_refusal(SyncSessionStatus::Resolved, false, None).is_some());
    assert_eq!(
        resync_refusal(SyncSessionStatus::Resolved, true, None),
        None,
        "a resolution origin already has is nothing to protect"
    );
    for status in [
        SyncSessionStatus::Syncing,
        SyncSessionStatus::UpToDate,
        SyncSessionStatus::Merged,
        SyncSessionStatus::Blocked,
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::ResolutionFailed,
        SyncSessionStatus::Aborted,
    ] {
        assert_eq!(
            resync_refusal(status, false, None),
            None,
            "{status:?} holds no committed resolution and must not block a sync"
        );
    }
}

/// The second row shape that carries a committed merge nobody has published,
/// and the one the refusal above did not cover.
///
/// A `push`-blocked session holds the merge on the branch plus the only copy of
/// `head_before` and `merge_commit_sha`. `open` is an upsert, so the next sync
/// takes all three; the merge it then runs finds `origin/<base>` already in the
/// branch, changes nothing, and lands the row on a terminal `up_to_date` from
/// which Publish is refused — leaving a merge on the local branch that the pull
/// request will never see. The pane withholds the retry, but the IPC behind it
/// stays reachable and the `merge` stage produces the same row shape while
/// offering one.
#[test]
fn a_committed_merge_that_never_reached_origin_is_not_something_to_sync_over() {
    use crate::domain::sync_session::resync_refusal;

    assert!(
        resync_refusal(
            SyncSessionStatus::Blocked,
            false,
            Some(SyncBlockedStage::Push)
        )
        .is_some(),
        "the only copy of head_before and the merge commit is on this row"
    );
    assert_eq!(
        resync_refusal(
            SyncSessionStatus::Blocked,
            true,
            Some(SyncBlockedStage::Push)
        ),
        None,
        "a merge origin already has is nothing to protect"
    );
    for stage in [
        SyncBlockedStage::Fetch,
        SyncBlockedStage::BaseRefMissing,
        SyncBlockedStage::WorktreeProvision,
        SyncBlockedStage::Merge,
        SyncBlockedStage::RepoContext,
        SyncBlockedStage::HeldResolution,
        SyncBlockedStage::TurnInFlight,
    ] {
        assert_eq!(
            resync_refusal(SyncSessionStatus::Blocked, false, Some(stage)),
            None,
            "{stage:?} merged nothing, so the next sync must not be refused"
        );
    }
}

/// The status a turn holds its worktree under is not the status it wrote.
///
/// `feature_resolve_sync_conflicts` claims the slot, then runs a preflight of
/// several round trips before `record_sync_resolution(Started)` — and that
/// write is best-effort, so a contended one leaves the row on `conflicted` for
/// the whole turn. Judged on status alone, every one of those windows offered
/// Abort: `git merge --abort`, `worktree remove --force`, `remove_dir_all`, at
/// the directory the agent is editing in. `reconcile` guards `syncing` and
/// `resolving` and nothing else, which is why the observation has to reach the
/// refusals too rather than only the correction.
#[test]
fn nothing_is_offered_while_a_turn_is_holding_the_worktree() {
    use crate::domain::sync_session::{intervention_refusal, SyncIntervention, SyncLiveness};

    for status in [
        SyncSessionStatus::Syncing,
        SyncSessionStatus::Blocked,
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::Resolving,
        SyncSessionStatus::Resolved,
        SyncSessionStatus::ResolutionFailed,
    ] {
        let held = SyncStanding {
            status,
            published: false,
            blocked_stage: Some(SyncBlockedStage::Push),
            feature_status: "completed",
            liveness: SyncLiveness::Live,
        };
        assert!(!user_may_intervene(held), "{status:?}");
        for action in [
            SyncIntervention::Abort,
            SyncIntervention::Resolve,
            SyncIntervention::Publish,
            SyncIntervention::Discard,
        ] {
            let refusal = intervention_refusal(action, held)
                .unwrap_or_else(|| panic!("{action:?} was offered on a live {status:?}"));
            assert!(refusal.contains("already running"), "{refusal}");
        }
    }
}

/// The opposite failure, and the one that makes the guard above safe to have.
///
/// A resolver that died leaves the same row a live one does, and the registry
/// is process-local precisely so a restart answers `Gone` for it. If liveness
/// ever outlived the turn, a conflict would be frozen with all four actions
/// refused and no way back.
#[test]
fn a_conflict_whose_turn_is_over_is_the_users_again() {
    use crate::domain::sync_session::SyncLiveness;

    for status in [
        SyncSessionStatus::Conflicted,
        SyncSessionStatus::ResolutionFailed,
    ] {
        assert!(
            user_may_intervene(SyncStanding {
                status,
                published: false,
                blocked_stage: None,
                feature_status: "completed",
                liveness: SyncLiveness::Gone,
            }),
            "{status:?}"
        );
    }
}
