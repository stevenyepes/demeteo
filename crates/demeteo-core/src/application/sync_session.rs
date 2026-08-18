//! Reading and ending a feature's live sync.
//!
//! Free functions over the two ports they need rather than methods on the step
//! executor: neither reads a workflow, a driver or an agent, and AGENTS.md §3
//! makes the narrower dependency the testable one — everything here is
//! reachable with a strict [`ExecutionPort`] double and an in-memory database.
//!
//! Both functions treat the working tree as the authority
//! ([`crate::ports::sync_session`]). The probe is here because it does I/O;
//! the decision it feeds is in [`crate::domain::sync_session::reconcile`].

use std::sync::Arc;

use crate::adapters::step_executor::sync_worktree::discard_sync_worktree;
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::domain::ids::FeatureId;
use crate::domain::sync_session::{
    intervention_refusal, reconcile, user_may_intervene, SyncIntervention, SyncSessionStatus,
    SyncStanding, SyncWorkspaceProbe,
};
use crate::paths;
use crate::ports::execution::{ask, Answer, ExecutionPort};
use crate::ports::sync_session::{SyncSession, SyncSessionPatch, SyncSessionPort, SyncSessionView};

/// The feature's session as the working tree says it stands.
///
/// The stored status is corrected before it is returned, and the correction is
/// persisted, so the next reader and the UI cannot disagree about a session
/// whose worktree has since gone.
pub async fn get_reconciled(
    sessions: &Arc<dyn SyncSessionPort>,
    exec: &Arc<dyn ExecutionPort>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSession>, String> {
    let Some(mut session) = sessions.get(feature_id)? else {
        return Ok(None);
    };
    // A session naming no worktree and a worktree that would not answer arrive
    // at `reconcile` as the same `None`, and that is deliberate: neither is an
    // observation, so neither may move the stored status.
    let probe = match session.worktree_path.as_deref() {
        Some(path) => {
            probe_worktree(
                &**exec,
                &session.machine_id,
                path,
                session.head_before.as_deref(),
            )
            .await
        }
        None => None,
    };
    let corrected = reconcile(session.status, probe.as_ref());
    if corrected != session.status {
        let now = paths::now_ms();
        sessions.update(
            feature_id,
            &SyncSessionPatch {
                status: Some(corrected),
                ..Default::default()
            },
            now,
        )?;
        session.status = corrected;
        session.updated_at = now;
    }
    Ok(Some(session))
}

/// The session plus the one question the UI may not answer for itself: whether
/// this sync is the user's to abort or re-resolve, or something else's.
pub async fn get_reconciled_view(
    sessions: &Arc<dyn SyncSessionPort>,
    exec: &Arc<dyn ExecutionPort>,
    features: &Arc<dyn crate::ports::db::FeatureRepository>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSessionView>, String> {
    let Some(session) = get_reconciled(sessions, exec, feature_id).await? else {
        return Ok(None);
    };
    // A feature the row outlived is not a run holding anything.
    let feature_status = feature_status_of(features, feature_id)?;
    Ok(Some(view(session, &feature_status)))
}

/// The row and the feature's status, in the shape every refusal is judged
/// against.
fn standing<'a>(session: &SyncSession, feature_status: &'a str) -> SyncStanding<'a> {
    SyncStanding {
        status: session.status,
        published: session.pushed_at.is_some(),
        feature_status,
    }
}

/// Send a resolution that is only on the feature branch to origin.
///
/// Pressing this twice is not an error and does not push twice: a session that
/// already carries a `pushed_at` answers with itself and issues no command at
/// all. That matters more than it looks — the affordance sits next to a diff
/// the user is reading, and the honest reading of a second press is "did the
/// first one work", which a refusal would answer with an error message.
///
/// **Nothing is recorded on the strength of the push's own exit code.** `git
/// push` exiting zero is a verdict, but the state that matters is whether the
/// commit is *on origin*, and the two come apart in both directions: a push
/// that was rejected because the branch moved under it, and a push that landed
/// on a connection that then died before saying so. So the remote-tracking ref
/// is asked afterwards whether it contains the commit, and only that answer
/// writes the column — the same rule [`abort`] applies to the worktree it
/// claims to have deleted.
///
/// Containment rather than equality, because the branch may legitimately have
/// grown since: a follow-up commit the user made themselves is still a branch
/// whose resolution reached origin, and demanding the tip *be* the merge commit
/// would refuse to record a push that plainly happened, forever.
pub async fn publish(
    sessions: &Arc<dyn SyncSessionPort>,
    exec: &Arc<dyn ExecutionPort>,
    features: &Arc<dyn crate::ports::db::FeatureRepository>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSessionView>, String> {
    let Some(session) = get_reconciled(sessions, exec, feature_id).await? else {
        return Ok(None);
    };
    let feature_status = feature_status_of(features, feature_id)?;
    if session.pushed_at.is_some() {
        return Ok(Some(view(session, &feature_status)));
    }
    if let Some(refusal) = intervention_refusal(
        SyncIntervention::Publish,
        standing(&session, &feature_status),
    ) {
        return Err(refusal.to_string());
    }
    let Some(sha) = session.merge_commit_sha.clone() else {
        return Err(
            "This sync recorded no resolution commit, so there is nothing to publish.".to_string(),
        );
    };
    // From the clone rather than the sync worktree. Linked worktrees share the
    // refs and the remotes, so both push the same branch — but the worktree is
    // the throwaway, and it is gone the moment this succeeds.
    let repo = paths::shell_escape_posix(&session.repo_dir);
    let branch = paths::shell_escape_posix(&session.feature_branch);
    if let Err(e) = exec
        .run_command(
            &session.machine_id,
            &format!("git -C {} push origin {}", repo, branch),
        )
        .await
    {
        return Err(match classify_exec_failure(&e) {
            HarnessExecFailure::NonZeroExit => format!(
                "origin refused the push. The branch may have moved since the resolution was \
                 made — fetch and sync again before publishing.\n\n{}",
                e
            ),
            HarnessExecFailure::Transport | HarnessExecFailure::Timeout => format!(
                "The push to origin/{} never reached a verdict, so the resolution is still \
                 unpublished: {}",
                session.feature_branch, e
            ),
        });
    }
    match ask(
        &**exec,
        &session.machine_id,
        &format!(
            "git -C {} merge-base --is-ancestor {} refs/remotes/origin/{}",
            repo,
            paths::shell_escape_posix(&sha),
            branch
        ),
    )
    .await
    {
        Answer::Said(_) => {}
        Answer::Refused => {
            return Err(format!(
                "The push reported success, but origin/{} does not contain the resolution \
                 commit. Nothing was recorded as published.",
                session.feature_branch
            ))
        }
        Answer::Unreadable(e) => {
            return Err(format!(
                "The push reported success but could not be confirmed against origin/{}, so it \
                 was not recorded. Press Publish again once the machine answers: {}",
                session.feature_branch, e
            ))
        }
    }

    let now = paths::now_ms();
    sessions.update(
        feature_id,
        &SyncSessionPatch {
            pushed_at: Some(Some(now)),
            ..Default::default()
        },
        now,
    )?;
    let mut published = session;
    published.pushed_at = Some(now);
    published.updated_at = now;

    // The tree was kept only so the resolution could be looked at and, if it
    // came to it, undone in the checkout that holds the branch. Published,
    // there is nothing left to do either of, so it goes — on the same
    // confirm-before-you-record rule as everywhere else, and best-effort:
    // failing to delete a directory is not a reason to tell the user their
    // push did not happen.
    if let Some(worktree) = published.worktree_path.clone() {
        if worktree != published.repo_dir {
            discard_sync_worktree(
                &**exec,
                &published.machine_id,
                &published.repo_dir,
                &worktree,
            )
            .await;
            if worktree_confirmed_gone(&**exec, &published.machine_id, &worktree).await {
                sessions.update(
                    feature_id,
                    &SyncSessionPatch {
                        worktree_path: Some(None),
                        ..Default::default()
                    },
                    now,
                )?;
                published.worktree_path = None;
            }
        }
    }
    Ok(Some(view(published, &feature_status)))
}

/// Throw the resolution away: put the feature branch back where the merge found
/// it, then abandon the sync.
///
/// **What the user gets back is an abandoned sync, not the conflict.** Undoing
/// the merge commit is a branch move; reproducing the conflicted tree would mean
/// re-running the merge, which is a different operation with a different outcome
/// (origin has moved since, and the conflict may not be the same one). Saying
/// "back to the conflict" and delivering this is the promise this refuses to
/// make — sync again to get a fresh one.
///
/// A session with no `head_before` is refused rather than guessed at.
/// `merge_commit^` names the pre-merge tip only until the resolver adds a
/// follow-up commit, and there is nothing on the row to tell the two apart, so
/// the guess would silently reset the branch to the wrong place exactly when
/// the resolution was most involved.
pub async fn discard_resolution(
    sessions: &Arc<dyn SyncSessionPort>,
    exec: &Arc<dyn ExecutionPort>,
    features: &Arc<dyn crate::ports::db::FeatureRepository>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSession>, String> {
    let Some(session) = get_reconciled(sessions, exec, feature_id).await? else {
        return Ok(None);
    };
    let feature_status = feature_status_of(features, feature_id)?;
    if let Some(refusal) = intervention_refusal(
        SyncIntervention::Discard,
        standing(&session, &feature_status),
    ) {
        return Err(refusal.to_string());
    }
    let Some(head_before) = session.head_before.clone() else {
        return Err(
            "This sync never recorded where the branch was before the merge, so the resolution \
             cannot be undone without guessing at it. Publish it, or move the branch yourself."
                .to_string(),
        );
    };
    // The reset has to run in a checkout that has the feature branch on HEAD,
    // and the sync worktree is the only place guaranteed to. Without one, the
    // clone is on whatever it was left on, and a `reset --hard` there would
    // move that branch instead.
    let Some(worktree) = session.worktree_path.clone() else {
        return Err(
            "The worktree this resolution was made in is gone, so there is nothing here to undo \
             it in. Publish it, or move the branch back yourself."
                .to_string(),
        );
    };
    let safe = paths::shell_escape_posix(&worktree);
    if let Err(e) = exec
        .run_command(
            &session.machine_id,
            &format!(
                "git -C {} reset --hard {}",
                safe,
                paths::shell_escape_posix(&head_before)
            ),
        )
        .await
    {
        return Err(format!(
            "Could not move {} back to {}, so nothing was discarded: {}",
            session.feature_branch, head_before, e
        ));
    }
    match ask(
        &**exec,
        &session.machine_id,
        &format!("git -C {} rev-parse HEAD", safe),
    )
    .await
    {
        Answer::Said(head) if head.trim() == head_before => {}
        Answer::Said(head) => {
            return Err(format!(
                "{} is at {} after the reset, not {}, so the sync was left alone.",
                session.feature_branch,
                head.trim(),
                head_before
            ))
        }
        Answer::Refused | Answer::Unreadable(_) => {
            return Err(format!(
                "Could not read {} back to confirm the branch moved, so the sync was left as it \
                 was. Try again once the machine answers.",
                worktree
            ))
        }
    }
    // The branch is back; what remains is the teardown and the verdict, which
    // `abort` already owns on exactly the terms this needs. Its refusal to
    // record a sync as abandoned while its tree may still be on disk applies
    // here too, and pressing Discard again re-runs a reset that is now a no-op.
    abort(sessions, exec, feature_id).await
}

fn feature_status_of(
    features: &Arc<dyn crate::ports::db::FeatureRepository>,
    feature_id: &FeatureId,
) -> Result<String, String> {
    Ok(features
        .get(feature_id)?
        .map(|f| f.status)
        .unwrap_or_default())
}

fn view(session: SyncSession, feature_status: &str) -> SyncSessionView {
    SyncSessionView {
        user_may_intervene: user_may_intervene(standing(&session, feature_status)),
        session,
    }
}

/// Abandon the feature's sync: undo the merge, discard the worktree, and mark
/// the session aborted.
///
/// The teardown steps stay best-effort — the common case is a tree that is
/// already gone, because the user aborts after a restart or after cleaning up by
/// hand — but the *verdict* is not taken from them. It is taken from a probe
/// afterwards, and the session is only closed once the tree is confirmed gone.
///
/// The difference shows up when the machine is unreachable. Recording `aborted`
/// on an unreachable host would tell the user the sync was abandoned while the
/// merge stayed open on disk, and because [`SyncSessionStatus::is_terminal`]
/// covers `aborted` and this is the call that clears `worktree_path`, the
/// orphaned tree would then be named by nothing and revisited by no reader —
/// leaving only the next sync's force-remove to reclaim it, which is the leak
/// V43 exists to close. So an unreadable tree is an `Err` and the row is left
/// exactly as it was, still pointing at the directory.
///
/// The delete goes through [`discard_sync_worktree`] rather than being
/// re-spelled, because `worktree_path` may legitimately be the clone itself
/// (`provision_sync_worktree` returns it when the feature branch is already
/// checked out there) and that function is where the guard lives.
pub async fn abort(
    sessions: &Arc<dyn SyncSessionPort>,
    exec: &Arc<dyn ExecutionPort>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSession>, String> {
    let Some(mut session) = sessions.get(feature_id)? else {
        return Ok(None);
    };
    if let Some(worktree) = session.worktree_path.as_deref() {
        let _ = exec
            .run_command(
                &session.machine_id,
                &format!(
                    "git -C {} merge --abort",
                    paths::shell_escape_posix(worktree)
                ),
            )
            .await;
        discard_sync_worktree(&**exec, &session.machine_id, &session.repo_dir, worktree).await;

        // `discard_sync_worktree` reports nothing by design, so ask git.
        // `worktree == repo_dir` is the one case where the tree legitimately
        // survives: the guard inside `discard_sync_worktree` refuses to delete
        // the clone, and undoing the merge there is the whole of the abort.
        if worktree != session.repo_dir
            && !worktree_confirmed_gone(&**exec, &session.machine_id, worktree).await
        {
            return Err(format!(
                "Could not confirm the sync worktree at {} is gone, so the sync is still open. \
                 The machine may be unreachable — try again once it answers.",
                worktree
            ));
        }
    }
    let now = paths::now_ms();
    sessions.update(
        feature_id,
        &SyncSessionPatch {
            status: Some(SyncSessionStatus::Aborted),
            worktree_path: Some(None),
            ..Default::default()
        },
        now,
    )?;
    session.status = SyncSessionStatus::Aborted;
    session.worktree_path = None;
    session.updated_at = now;
    Ok(Some(session))
}

/// Has `worktree` been observed to be gone — as opposed to merely failing to
/// answer?
///
/// The one question every path that *clears* `worktree_path` has to answer
/// first, and the reason [`abort`] refuses rather than closing an unreadable
/// session. A resolution's teardown owes the same care for the same reason:
/// [`discard_sync_worktree`] reports nothing, so a row blanked on the strength
/// of a delete nobody confirmed names a directory that may still be on disk,
/// and no reader is left to revisit it.
pub(crate) async fn worktree_confirmed_gone(
    exec: &dyn ExecutionPort,
    machine: &str,
    worktree: &str,
) -> bool {
    matches!(
        probe_worktree(exec, machine, worktree, None).await,
        Some(probe) if !probe.worktree_exists
    )
}

/// What the tree at `worktree` says about the merge the session claims, or
/// `None` when it could not be read.
///
/// The difference between those two is the whole point. `git -C <missing>`
/// exits non-zero, so a refused `rev-parse --git-dir` really does mean the
/// directory is gone — but a dropped channel and an expired deadline reach the
/// caller as errors too, and they are not answers. Reading one as "the tree is
/// gone" marks a live conflict [`SyncSessionStatus::Aborted`], which is
/// terminal, so on the SSH transport a single lost keepalive would destroy the
/// record this table exists to keep. [`crate::domain::sync_failure`] refuses
/// the same inference at the same boundary for the same reason.
///
/// Every negative here has to be a verdict rather than an absence, which is why
/// `MERGE_HEAD` and the porcelain are read through [`Answer`] as well: an
/// unreadable `MERGE_HEAD` presented as "no merge open" sends
/// [`reconcile`] down its resolved arm.
async fn probe_worktree(
    exec: &dyn ExecutionPort,
    machine: &str,
    worktree: &str,
    head_before: Option<&str>,
) -> Option<SyncWorkspaceProbe> {
    let safe = paths::shell_escape_posix(worktree);
    match ask(
        exec,
        machine,
        &format!("git -C {} rev-parse --git-dir", safe),
    )
    .await
    {
        Answer::Refused => {
            return Some(SyncWorkspaceProbe {
                worktree_exists: false,
                merge_in_progress: false,
                dirty: false,
                head_advanced: None,
            })
        }
        Answer::Unreadable(_) => return None,
        Answer::Said(_) => {}
    }
    let merge_in_progress = match ask(
        exec,
        machine,
        &format!("git -C {} rev-parse --verify --quiet MERGE_HEAD", safe),
    )
    .await
    {
        // `--quiet` is what makes the refusal meaningful: no MERGE_HEAD exits 1
        // rather than printing a diagnostic.
        Answer::Said(out) => !out.trim().is_empty(),
        Answer::Refused => false,
        Answer::Unreadable(_) => return None,
    };
    let dirty = match ask(
        exec,
        machine,
        &format!("git -C {} status --porcelain", safe),
    )
    .await
    {
        Answer::Said(out) => !out.trim().is_empty(),
        // A repository that answered `--git-dir` and then refused a status read
        // is not reporting a clean tree, it is not reporting.
        Answer::Refused | Answer::Unreadable(_) => return None,
    };
    // Only a closed merge over a clean tree is ambiguous, so that is the only
    // shape worth a fourth round trip — and asking unconditionally would put a
    // command in front of the strict test double that no assertion is about.
    let head_advanced = match (merge_in_progress, dirty, head_before) {
        (false, false, Some(before)) => {
            match ask(exec, machine, &format!("git -C {} rev-parse HEAD", safe)).await {
                Answer::Said(head) => Some(head.trim() != before),
                Answer::Refused | Answer::Unreadable(_) => None,
            }
        }
        _ => None,
    };
    Some(SyncWorkspaceProbe {
        worktree_exists: true,
        merge_in_progress,
        dirty,
        head_advanced,
    })
}

#[cfg(test)]
#[path = "../../tests/application/sync_session.rs"]
mod tests;
