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
    reconcile, user_may_intervene, SyncSessionStatus, SyncWorkspaceProbe,
};
use crate::paths;
use crate::ports::execution::ExecutionPort;
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
    let feature_status = features
        .get(feature_id)?
        .map(|f| f.status)
        .unwrap_or_default();
    Ok(Some(SyncSessionView {
        user_may_intervene: user_may_intervene(session.status, &feature_status),
        session,
    }))
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
            && !matches!(
                probe_worktree(&**exec, &session.machine_id, worktree, None).await,
                Some(probe) if !probe.worktree_exists
            )
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
        Answer::Unreadable => return None,
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
        Answer::Unreadable => return None,
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
        Answer::Refused | Answer::Unreadable => return None,
    };
    // Only a closed merge over a clean tree is ambiguous, so that is the only
    // shape worth a fourth round trip — and asking unconditionally would put a
    // command in front of the strict test double that no assertion is about.
    let head_advanced = match (merge_in_progress, dirty, head_before) {
        (false, false, Some(before)) => {
            match ask(exec, machine, &format!("git -C {} rev-parse HEAD", safe)).await {
                Answer::Said(head) => Some(head.trim() != before),
                Answer::Refused | Answer::Unreadable => None,
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

/// What one probe command came back with.
enum Answer {
    /// git ran and answered; the payload is its stdout.
    Said(String),
    /// git ran and refused. A negative result, and usable as one.
    Refused,
    /// The command never reached a verdict — the transport failed or the
    /// deadline expired. Nothing may be concluded from it in either direction.
    Unreadable,
}

async fn ask(exec: &dyn ExecutionPort, machine: &str, command: &str) -> Answer {
    match exec.run_command(machine, command).await {
        Ok(out) => Answer::Said(out),
        Err(e) => match classify_exec_failure(&e) {
            HarnessExecFailure::NonZeroExit => Answer::Refused,
            HarnessExecFailure::Transport | HarnessExecFailure::Timeout => Answer::Unreadable,
        },
    }
}

#[cfg(test)]
#[path = "../../tests/application/sync_session.rs"]
mod tests;
