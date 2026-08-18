//! Feature-branch sync with the upstream `base_branch`.
//!
//! Two Tauri commands surface this code path:
//!
//! - `feature_sync`: merges `origin/<base>` into the feature branch.
//!   A clean merge is a `SyncOutcomeView::Ok`; unmerged paths are a
//!   `SyncOutcomeView::Conflict`, which is the only outcome the UI offers
//!   "Resolve with agent" for. Everything that failed before the merge is a
//!   `SyncOutcomeView::Blocked` — [`crate::domain::sync_failure`] owns that
//!   split and `view_for` is the only thing that decides it.
//!
//! - `feature_resolve_sync_conflicts`: hands the conflict the merge left to
//!   [`crate::adapters::step_executor::sync_resolve`], which is the same turn
//!   the workflow `sync` node runs.
//!
//! Both commands live in `commands/features.rs` (the thin IPC
//! layer); this module owns the orchestration. It reuses the existing
//! `GitOpsHelper` for git, `MergeExecutor` for the conflict
//! detection, and the `AgentRegistry` for spawning — no new ports.

use crate::adapters::step_executor::steps::list_unmerged::list_unmerged_files;
use crate::adapters::step_executor::sync_resolve::{
    resolve_sync_conflicts, ResolveSyncContext, SYNC_RESOLVER_THREAD_PREFIX,
};
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::sync_session::intervention_refusal;
use crate::paths;
use crate::ports::step_executor::SyncOutcomeView;

use super::DagStepExecutor;

/// The branch a sync merges into the feature branch.
///
/// [`diff_base::resolve`](crate::domain::diff_base::resolve) and nothing
/// else: a run cut from `origin/release/2.0` that merged the project default
/// instead would pull the whole of trunk into a release branch's feature.
pub(crate) fn sync_base(
    feature: &crate::domain::models::Feature,
    settings: &crate::domain::models::ProjectSettings,
) -> Result<String, String> {
    crate::domain::diff_base::resolve(
        feature.diff_base_branch.as_deref(),
        &feature.origin,
        &settings.worktree_strategy.default_branch,
    )
    .map(str::to_string)
    .ok_or_else(|| {
        "This run names no base branch to sync from; set the project's default branch.".to_string()
    })
}

impl DagStepExecutor {
    /// Tauri entry point for the "Sync with main" command. Resolves
    /// the feature branch + project state, asks the merge executor to
    /// do the actual git work, and translates the result into a
    /// `SyncOutcomeView` for the UI.
    pub(crate) async fn feature_sync_impl(
        &self,
        feature_id: &str,
    ) -> Result<SyncOutcomeView, String> {
        let fid = FeatureId::from(feature_id.to_string());
        let feature = self
            .features
            .get(&fid)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let base_branch = sync_base(&feature, &settings)?;
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        Ok(crate::domain::sync_failure::view_for(
            self.merge_executor
                .sync_feature_with_upstream(&fid, &feature_branch, &base_branch)
                .await,
        ))
    }

    /// Tauri entry point for the "Resolve with agent" button.
    ///
    /// Every fact about *which* conflict this is comes off the feature's sync
    /// session: the worktree the merge left, the clone it was cut from, the
    /// machine, and both branch names. They were re-derived here until V43 gave
    /// the sync a row of its own — the worktree by string-searching the newest
    /// `conflict` row of the audit trail, which answered with a `worktree_path`
    /// of `None` as soon as a later attempt failed before merging, and then fell
    /// back to checking the feature branch out in the user's own clone.
    ///
    /// The refusal is the other half of having a row: a conflict a run is
    /// resolving is visible to this IPC as readily as to the button, and putting
    /// a second agent in a worktree an agent already holds is worse than doing
    /// nothing.
    pub(crate) async fn feature_resolve_sync_conflicts_impl(
        &self,
        feature_id: &str,
        conflict_files: &[String],
    ) -> Result<SyncOutcomeView, String> {
        let fid = FeatureId::from(feature_id.to_string());
        let feature = self
            .features
            .get(&fid)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        let session = self
            .merge_executor
            .sync_session(&fid)
            .await?
            .ok_or_else(|| {
                "This feature has no sync to resolve. Run 'Sync with main' first.".to_string()
            })?;
        if let Some(refusal) = intervention_refusal(session.status, &feature.status) {
            return Err(refusal.to_string());
        }
        let worktree = session.worktree_path.as_deref().ok_or_else(|| {
            "This sync never provisioned a worktree, so there is nothing to resolve in one."
                .to_string()
        })?;

        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        let agent_kind = feature
            .agent_kind
            .clone()
            .unwrap_or_else(|| "opencode".to_string());
        let override_model = feature.model.clone();
        // No driver is running here (this is the "Resolve with agent" button),
        // so walk what the feature row + project settings know: the run
        // override, then the project default, then the built-in high.
        let effort = feature
            .effort
            .or(settings.default_effort)
            .unwrap_or(crate::domain::models::EffortLevel::DEFAULT);

        let step_exec_id = StepExecutionId::from(format!("se-sync-{}", paths::now_ms()));
        match resolve_sync_conflicts(ResolveSyncContext {
            exec: &self.exec,
            registry: &self.registry,
            notif: &self.notif,
            agent_exec: &self.agent_exec,
            app_settings: &self.app_settings,
            git_ops: &self.git_ops,
            merge_executor: &self.merge_executor,
            feature_id: &fid,
            repo_dir: &session.repo_dir,
            resolved_cwd: worktree,
            machine_str: &session.machine_id,
            feature_branch: &session.feature_branch,
            base_branch: &session.base_branch,
            conflict_files,
            step_execution_id: &step_exec_id,
            thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
            agent_kind: &agent_kind,
            override_model: override_model.as_deref(),
            effort,
            pricing: &self.pricing,
        })
        .await
        {
            Ok(merge_commit_sha) => Ok(SyncOutcomeView::Resolved { merge_commit_sha }),
            Err(reason) => Ok(SyncOutcomeView::ResolutionFailed {
                reason,
                conflict_files: list_unmerged_files(&*self.exec, &session.machine_id, worktree)
                    .await,
            }),
        }
    }
}
