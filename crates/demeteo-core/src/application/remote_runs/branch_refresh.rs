//! Telling the runner that owns a detached run that a desktop sync put
//! something on origin, and telling the user when it could not take it. See
//! [`crate::domain::runner_branch_refresh`].
//!
//! The sync's own result never depends on any of this: it has already reached
//! origin by the time the runner is asked, and the runner's answer is reported
//! beside it, through the bell, rather than folded into it.

use std::future::Future;

use super::credentials::pat_for_run;
use super::rpc::remote_rpc_via;
use crate::domain::ids::FeatureId;
use crate::domain::models::{Notification, NotificationKind};
use crate::domain::runner_branch_refresh::{
    newly_on_origin, refresh_notice, BranchRefreshOutcome, SyncMark, METHOD,
};
use crate::ports::db::{AppSettingsRepository, FeatureRepository, NotificationRepository};
use crate::ports::execution::ExecutionPort;
use crate::ports::notification::{DomainEvent, NotificationPort};
use crate::ports::remote_run_mirror::{RemoteRunMirrorPort, RunnerBranchPort};
use crate::ports::sync_session::SyncSession;
use crate::state::AppContext;
use async_trait::async_trait;
use std::sync::Arc;

pub struct RunnerBranchRpc {
    pub exec: Arc<dyn ExecutionPort>,
    pub app_settings: Arc<dyn AppSettingsRepository>,
}

#[async_trait]
impl RunnerBranchPort for RunnerBranchRpc {
    async fn refresh_feature_branch(
        &self,
        machine_id: &str,
        run_id: &str,
        git_pat: Option<&str>,
    ) -> Result<BranchRefreshOutcome, String> {
        let answer = remote_rpc_via(
            &*self.exec,
            &*self.app_settings,
            machine_id,
            METHOD,
            serde_json::json!({ "run_id": run_id, "git_pat": git_pat }),
        )
        .await?;
        serde_json::from_value(answer)
            .map_err(|e| format!("the runner's answer to {METHOD} was unreadable: {e}"))
    }
}

/// The ports [`refresh_runner_branch`] reads and writes.
pub struct BranchRefreshPorts<'a> {
    pub mirrors: &'a dyn RemoteRunMirrorPort,
    pub runner: &'a dyn RunnerBranchPort,
    pub features: &'a dyn FeatureRepository,
    pub notifications: &'a dyn NotificationRepository,
    pub notif: &'a dyn NotificationPort,
}

/// Ask the runner holding `feature_id`'s detached run to bring its branch up to
/// date, and put any answer short of that in the bell. Returns what the user
/// was told; `None` for a feature no runner holds, or a refresh that landed.
pub async fn refresh_runner_branch(
    ports: &BranchRefreshPorts<'_>,
    feature_id: &FeatureId,
    branch: &str,
    git_pat: Option<&str>,
) -> Result<Option<String>, String> {
    let Some(run) = ports
        .mirrors
        .list_for_features(&[feature_id.as_str()])?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let result = ports
        .runner
        .refresh_feature_branch(&run.machine_id, &run.run_id, git_pat)
        .await;
    let Some(message) = refresh_notice(branch, &result) else {
        return Ok(None);
    };
    let project_id = match ports.features.get(feature_id)? {
        Some(feature) => feature.project_id.0,
        None => run.project_id.clone().unwrap_or_default(),
    };
    let now = crate::paths::now_ms();
    ports.notifications.add(Notification {
        id: format!("notif-runner-branch-{}-{now}", feature_id.as_str()),
        project_id: project_id.clone(),
        feature_id: feature_id.0.clone(),
        kind: NotificationKind::RunnerBranchStale,
        message: message.clone(),
        feature_url: Some(format!(
            "/projects/{project_id}/features/{}",
            feature_id.as_str()
        )),
        read: false,
        created_at: now,
    })?;
    let _ = ports.notif.emit(&DomainEvent::RunnerBranchStale {
        project_id,
        feature_id: feature_id.0.clone(),
        message: message.clone(),
    });
    Ok(Some(message))
}

fn mark(session: &SyncSession) -> SyncMark {
    SyncMark {
        status: session.status,
        merge_commit_sha: session.merge_commit_sha.clone(),
        pushed_at: session.pushed_at,
    }
}

/// Run one sync command, then — when it put something on origin — refresh the
/// runner's branch in the background. `op`'s result is returned untouched.
pub async fn with_runner_branch_refresh<T, E>(
    ctx: &AppContext,
    feature_id: &FeatureId,
    op: impl Future<Output = Result<T, E>>,
) -> Result<T, E> {
    let read = || ctx.sync_sessions.get(feature_id).ok().flatten();
    let before = read().map(|s| mark(&s));
    let result = op.await;
    if result.is_ok() {
        if let Some(after) = read() {
            if newly_on_origin(before.as_ref(), Some(&mark(&after))) {
                tokio::spawn(refresh_after_sync(
                    ctx.clone(),
                    feature_id.clone(),
                    after.feature_branch,
                ));
            }
        }
    }
    result
}

async fn refresh_after_sync(ctx: AppContext, feature_id: FeatureId, branch: String) {
    let Ok(Some(run)) = ctx
        .remote_run_mirror
        .list_for_features(&[feature_id.as_str()])
        .map(|rows| rows.into_iter().next())
    else {
        return;
    };
    // A PAT that cannot be resolved is not a reason to skip the refresh: an
    // ssh or public origin fetches without one, and a private one fails in the
    // runner's own words, which reach the bell.
    let pat = pat_for_run(&ctx, &run).ok();
    let runner = RunnerBranchRpc {
        exec: ctx.exec.clone(),
        app_settings: ctx.app_settings.clone(),
    };
    let ports = BranchRefreshPorts {
        mirrors: &*ctx.remote_run_mirror,
        runner: &runner,
        features: &*ctx.features,
        notifications: &*ctx.notifications,
        notif: &*ctx.notif,
    };
    if let Err(error) = refresh_runner_branch(&ports, &feature_id, &branch, pat.as_deref()).await {
        eprintln!(
            "[RunnerBranch] could not report the refresh of {} for {}: {error}",
            branch,
            feature_id.as_str()
        );
    }
}

#[cfg(test)]
#[path = "../../../tests/application/remote_runs/branch_refresh.rs"]
mod tests;
