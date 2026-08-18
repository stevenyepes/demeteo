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
use crate::domain::ids::FeatureId;
use crate::domain::sync_session::{reconcile, SyncSessionStatus, SyncWorkspaceProbe};
use crate::paths;
use crate::ports::execution::ExecutionPort;
use crate::ports::sync_session::{SyncSession, SyncSessionPatch, SyncSessionPort};

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
    let probe = match session.worktree_path.as_deref() {
        Some(path) => Some(probe_worktree(&**exec, &session.machine_id, path).await),
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

/// Abandon the feature's sync: undo the merge, discard the worktree, and mark
/// the session aborted.
///
/// Every step is best-effort and the common case is a worktree that is already
/// gone — the user aborts after a restart, or after cleaning up by hand — so
/// nothing here fails on an absent tree. The delete goes through
/// [`discard_sync_worktree`] rather than being re-spelled, because
/// `worktree_path` may legitimately be the clone itself
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

/// What the tree at `worktree` says about the merge the session claims.
///
/// A directory that answers nothing is a directory that is gone: every probe
/// is a git invocation against it, and `git -C <missing>` fails rather than
/// answering emptily, so a failed `rev-parse --git-dir` is the existence test
/// without a second filesystem round trip.
async fn probe_worktree(
    exec: &dyn ExecutionPort,
    machine: &str,
    worktree: &str,
) -> SyncWorkspaceProbe {
    let safe = paths::shell_escape_posix(worktree);
    let worktree_exists = exec
        .run_command(machine, &format!("git -C {} rev-parse --git-dir", safe))
        .await
        .is_ok();
    if !worktree_exists {
        return SyncWorkspaceProbe {
            worktree_exists: false,
            merge_in_progress: false,
            dirty: false,
        };
    }
    let merge_in_progress = exec
        .run_command(
            machine,
            &format!("git -C {} rev-parse --verify --quiet MERGE_HEAD", safe),
        )
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false);
    let dirty = exec
        .run_command(machine, &format!("git -C {} status --porcelain", safe))
        .await
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false);
    SyncWorkspaceProbe {
        worktree_exists,
        merge_in_progress,
        dirty,
    }
}

#[cfg(test)]
#[path = "../../tests/application/sync_session.rs"]
mod tests;
