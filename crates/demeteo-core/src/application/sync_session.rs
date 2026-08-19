//! Reading and ending a feature's live sync.
//!
//! Free functions over the few ports they need rather than methods on the step
//! executor: none reads a workflow, a driver or an agent, and AGENTS.md §3
//! makes the narrower dependency the testable one — everything here is
//! reachable with a strict [`ExecutionPort`] double, an in-memory database and
//! an empty [`SyncTurns`].
//!
//! Everything here treats the working tree as the authority
//! ([`crate::ports::sync_session`]). The probe is here because it does I/O;
//! the decision it feeds is in [`crate::domain::sync_session::reconcile`].

use std::sync::Arc;

use crate::adapters::step_executor::sync_worktree::discard_sync_worktree;
use crate::application::sync_turns::SyncTurns;
use crate::domain::harness_failure::{classify_exec_failure, HarnessExecFailure};
use crate::domain::ids::FeatureId;
use crate::domain::sync_session::{
    intervention_refusal, published_status, reconcile, sync_liveness, user_may_intervene,
    SyncIntervention, SyncSessionStatus, SyncStanding, SyncWorkspaceProbe,
};
use crate::paths;
use crate::ports::execution::{ask, Answer, ExecutionPort};
use crate::ports::sync_session::{SyncSession, SyncSessionPatch, SyncSessionPort, SyncSessionView};

/// The ports every read here needs, bundled because none of them answers on
/// its own: the row is the claim, the working tree and the turn registry are
/// the two observations that correct it, and the feature's status is half of
/// the second one (AGENTS.md §3 on parameters that travel together).
#[derive(Clone)]
pub struct SyncPorts<'a> {
    pub sessions: &'a Arc<dyn SyncSessionPort>,
    pub exec: &'a Arc<dyn ExecutionPort>,
    pub features: &'a Arc<dyn crate::ports::db::FeatureRepository>,
    pub turns: &'a Arc<SyncTurns>,
}

/// The feature's session as the working tree says it stands.
///
/// The stored status is corrected before it is returned, and the correction is
/// persisted, so the next reader and the UI cannot disagree about a session
/// whose worktree has since gone.
pub async fn get_reconciled(
    ports: SyncPorts<'_>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSession>, String> {
    Ok(reconciled(ports, feature_id).await?.map(|r| r.session))
}

/// What one look at a sync worktree came back with.
///
/// The sha itself rides beside the probe because two readers need it and not
/// only whether it moved: a session promoted to
/// [`SyncSessionStatus::Resolved`] out of the tree alone has no other source
/// for the commit that proves it, and `discard_resolution` has to see the
/// branch still standing on that commit before it resets. Asking again would
/// be a second round trip for a sha this read already holds.
struct WorktreeReading {
    probe: SyncWorkspaceProbe,
    head: Option<String>,
}

/// A session read back, with everything the read already established: no caller
/// needs to ask the feature repository a second time for a status this had to
/// have in order to reconcile at all.
struct Reconciled {
    session: SyncSession,
    reading: Option<WorktreeReading>,
    feature_status: String,
}

/// [`get_reconciled`], keeping the observations the correction was made from.
async fn reconciled(
    ports: SyncPorts<'_>,
    feature_id: &FeatureId,
) -> Result<Option<Reconciled>, String> {
    let SyncPorts {
        sessions,
        exec,
        features,
        turns,
    } = ports;
    let Some(mut session) = sessions.get(feature_id)? else {
        return Ok(None);
    };
    // A feature the row outlived is not a run holding anything.
    let feature_status = feature_status_of(features, feature_id)?;
    let liveness = sync_liveness(turns.claimed(&feature_id.0), &feature_status);
    // A session naming no worktree and a worktree that would not answer arrive
    // at `reconcile` as the same `None`, and that is deliberate: neither is an
    // observation, so neither may move the stored status.
    let reading = match session.worktree_path.as_deref() {
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
    let corrected = reconcile(session.status, reading.as_ref().map(|r| &r.probe), liveness);
    // The user who finished the merge in their own editor is the case this
    // module opens on, and it is the one resolution nothing recorded a commit
    // for: `reconcile` promotes it out of a moved `HEAD` alone. Left unwritten,
    // the review card offers a Publish that can only answer "this sync recorded
    // no resolution commit".
    let observed_commit = match reading.as_ref() {
        Some(r) if r.probe.head_advanced == Some(true) => r.head.clone(),
        _ => None,
    };
    let commit = match (corrected.status, &session.merge_commit_sha) {
        (SyncSessionStatus::Resolved, None) => observed_commit,
        _ => None,
    };
    if corrected.status != session.status || commit.is_some() {
        let now = paths::now_ms();
        sessions.update(
            feature_id,
            &SyncSessionPatch {
                status: Some(corrected.status),
                blocked_stage: corrected.blocked_stage.map(Some),
                merge_commit_sha: commit.clone().map(Some),
                ..Default::default()
            },
            now,
        )?;
        session.status = corrected.status;
        if let Some(stage) = corrected.blocked_stage {
            session.blocked_stage = Some(stage);
        }
        if commit.is_some() {
            session.merge_commit_sha = commit;
        }
        session.updated_at = now;
    }
    Ok(Some(Reconciled {
        session,
        reading,
        feature_status,
    }))
}

/// The session plus the one question the UI may not answer for itself: whether
/// this sync is the user's to abort or re-resolve, or something else's.
pub async fn get_reconciled_view(
    ports: SyncPorts<'_>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSessionView>, String> {
    let Some(read) = reconciled(ports, feature_id).await? else {
        return Ok(None);
    };
    Ok(Some(view(read.session, &read.feature_status)))
}

/// The row and the feature's status, in the shape every refusal is judged
/// against.
fn standing<'a>(session: &SyncSession, feature_status: &'a str) -> SyncStanding<'a> {
    SyncStanding {
        status: session.status,
        published: session.pushed_at.is_some(),
        blocked_stage: session.blocked_stage,
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
    ports: SyncPorts<'_>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSessionView>, String> {
    let SyncPorts { sessions, exec, .. } = ports;
    let Some(read) = reconciled(ports.clone(), feature_id).await? else {
        return Ok(None);
    };
    let (session, feature_status) = (read.session, read.feature_status);
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
    match push_landed(
        &**exec,
        &session.machine_id,
        &session.repo_dir,
        &session.feature_branch,
        &sha,
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
    // A `Push`-blocked sync is not what it was the moment origin has the merge:
    // the failed push *was* the block, and nothing about it was ever conflicted
    // ([`published_status`]). Leaving it `blocked` would keep a finished sync
    // offering an abandon, and keep the row naming a stage that no longer
    // describes anything.
    let status = published_status(session.status);
    let promoted = status != session.status;
    sessions.update(
        feature_id,
        &SyncSessionPatch {
            pushed_at: Some(Some(now)),
            status: promoted.then_some(status),
            blocked_stage: promoted.then_some(None),
            ..Default::default()
        },
        now,
    )?;
    let mut published = session;
    published.pushed_at = Some(now);
    published.updated_at = now;
    if promoted {
        published.status = status;
        published.blocked_stage = None;
    }

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
///
/// **The branch has to still be standing on the resolution, and the tree has to
/// be clean.** `reset --hard` destroys whatever is between here and
/// `head_before` without asking, and the checkout it runs in can be the user's
/// own clone: `provision_sync_worktree` returns `repo_dir` when the feature
/// branch is already checked out there, and a held resolution deliberately
/// leaves that value on the row. So a commit somebody added on top, and any
/// uncommitted work in that checkout, each refuse the discard rather than being
/// thrown away by it — the same evidence-before-record rule [`publish`] applies
/// to origin, pointed the other way.
pub async fn discard_resolution(
    ports: SyncPorts<'_>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSessionView>, String> {
    let SyncPorts { sessions, exec, .. } = ports;
    let Some(read) = reconciled(ports.clone(), feature_id).await? else {
        return Ok(None);
    };
    let Reconciled {
        session,
        reading,
        feature_status,
    } = read;
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
    let Some(resolution) = session.merge_commit_sha.clone() else {
        return Err(
            "This sync recorded no resolution commit, so there is nothing here to identify what \
             would be undone. Move the branch back yourself if that is what you want."
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
    let Some(reading) = reading else {
        return Err(format!(
            "Could not read {} to confirm what is there, so nothing was discarded. Try again \
             once the machine answers.",
            worktree
        ));
    };
    if reading.probe.dirty {
        return Err(format!(
            "There are uncommitted changes in {}, and moving the branch back would throw them \
             away. Nothing was discarded — commit or clean them up first.",
            worktree
        ));
    }
    match reading.head.as_deref() {
        Some(head) if head == resolution => {}
        Some(head) => {
            return Err(format!(
                "{} is at {}, not the resolution {}, so something has moved it since. Discarding \
                 would throw that away, and nothing was done.",
                session.feature_branch, head, resolution
            ))
        }
        None => {
            return Err(format!(
                "Could not read {} back to confirm it is still on the resolution, so nothing was \
                 discarded. Try again once the machine answers.",
                session.feature_branch
            ))
        }
    }
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
    // `close_session` already owns on exactly the terms this needs. Its refusal
    // to record a sync as abandoned while its tree may still be on disk applies
    // here too. `abort` itself is not what runs: this session is `resolved`,
    // which is the one standing that entry point must turn away.
    let closed = close_session(sessions, exec, session).await?;
    Ok(Some(view(closed, &feature_status)))
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
/// **What this may be asked of is [`intervention_refusal`]'s answer, not the
/// caller's.** The UI hides the button, but `sync_abort` stays reachable, and
/// the standing it must turn away is a committed resolution: this path undoes
/// an *open* merge, so aimed at one it deletes the tree, records the sync as
/// abandoned and leaves the merge on the branch with `Publish` and `Discard`
/// both refused afterwards. [`discard_resolution`] is the one that moves the
/// branch, and it reaches the teardown below without coming through here.
///
/// The refusal is read against the *reconciled* status and not the stored one,
/// because the session abort exists for is precisely the one whose writer died:
/// a row left `resolving` by a killed resolver would otherwise be turned away
/// with "an agent is already resolving this sync". Which is exactly what a
/// resolver that is still alive *is* turned away with — the two rows are
/// identical and only
/// [`sync_liveness`](crate::domain::sync_session::sync_liveness) separates
/// them, which is why this is the one command that must never reconcile
/// without it. An already-`aborted` session
/// skips the refusal rather than being turned away by it — the common case is a
/// second press, or a first one after a restart, and an error dialog is the
/// wrong answer to "did that work".
pub async fn abort(
    ports: SyncPorts<'_>,
    feature_id: &FeatureId,
) -> Result<Option<SyncSessionView>, String> {
    let SyncPorts { sessions, exec, .. } = ports;
    let Some(read) = reconciled(ports.clone(), feature_id).await? else {
        return Ok(None);
    };
    let (session, feature_status) = (read.session, read.feature_status);
    if session.status != SyncSessionStatus::Aborted {
        if let Some(refusal) =
            intervention_refusal(SyncIntervention::Abort, standing(&session, &feature_status))
        {
            return Err(refusal.to_string());
        }
    }
    let closed = close_session(sessions, exec, session).await?;
    Ok(Some(view(closed, &feature_status)))
}

/// Undo whatever merge is open, remove the throwaway tree, and record the sync
/// as abandoned.
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
async fn close_session(
    sessions: &Arc<dyn SyncSessionPort>,
    exec: &Arc<dyn ExecutionPort>,
    mut session: SyncSession,
) -> Result<SyncSession, String> {
    let feature_id = &FeatureId::from(session.feature_id.clone());
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
    Ok(session)
}

/// Has `worktree` been observed to be gone — as opposed to merely failing to
/// answer?
///
/// The one question every path that *clears* `worktree_path` has to answer
/// first, and the reason [`close_session`] refuses rather than closing an
/// unreadable session. A resolution's teardown owes the same care for the same reason:
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
        Some(reading) if !reading.probe.worktree_exists
    )
}

/// Whether `sha` is reachable from `origin/<branch>` — the only evidence
/// either publisher is allowed to record a push on.
///
/// `git push` exiting zero is a verdict about the command and not about origin,
/// and the two come apart in both directions: a push rejected because the
/// branch moved under it, and a push that landed on a connection that died
/// before saying so. Containment rather than equality, because the branch may
/// legitimately have grown since — [`publish`] has the rest of that reasoning.
///
/// The answer is handed back whole rather than as a bool: `Refused` is origin
/// saying it does not have the commit, `Unreadable` is nobody saying anything,
/// and a caller that collapses them records a publication from a dead channel.
pub(crate) async fn push_landed(
    exec: &dyn ExecutionPort,
    machine: &str,
    repo_dir: &str,
    branch: &str,
    sha: &str,
) -> Answer {
    ask(
        exec,
        machine,
        &format!(
            "git -C {} merge-base --is-ancestor {} refs/remotes/origin/{}",
            paths::shell_escape_posix(repo_dir),
            paths::shell_escape_posix(sha),
            paths::shell_escape_posix(branch)
        ),
    )
    .await
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
) -> Option<WorktreeReading> {
    let safe = paths::shell_escape_posix(worktree);
    match ask(
        exec,
        machine,
        &format!("git -C {} rev-parse --git-dir", safe),
    )
    .await
    {
        Answer::Refused => {
            return Some(WorktreeReading {
                probe: SyncWorkspaceProbe {
                    worktree_exists: false,
                    merge_in_progress: false,
                    dirty: false,
                    head_advanced: None,
                },
                head: None,
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
    let head = match (merge_in_progress, dirty, head_before) {
        (false, false, Some(_)) => {
            match ask(exec, machine, &format!("git -C {} rev-parse HEAD", safe)).await {
                Answer::Said(head) => Some(head.trim().to_string()),
                Answer::Refused | Answer::Unreadable(_) => None,
            }
        }
        _ => None,
    };
    let head_advanced = match (&head, head_before) {
        (Some(head), Some(before)) => Some(head != before),
        _ => None,
    };
    Some(WorktreeReading {
        probe: SyncWorkspaceProbe {
            worktree_exists: true,
            merge_in_progress,
            dirty,
            head_advanced,
        },
        head,
    })
}

#[cfg(test)]
#[path = "../../tests/application/sync_session.rs"]
mod tests;
