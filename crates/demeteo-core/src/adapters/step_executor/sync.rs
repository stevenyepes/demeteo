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

use std::time::Instant;

use tokio::sync::watch;

use crate::adapters::step_executor::spend::RunningSpend;
use crate::adapters::step_executor::step_status::{
    update_step_status, CacheTokens, StatusWriters, StepTransition,
};
use crate::adapters::step_executor::steps::list_unmerged::list_unmerged_files;
use crate::adapters::step_executor::sync_resolve::{
    resolve_sync_conflicts, ResolveSyncContext, ResolveSyncError, SYNC_RESOLVER_THREAD_PREFIX,
};
use crate::domain::agent_session::budget;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::domain::step_seed::manual_sync_step_execution;
use crate::domain::sync_session::intervention_refusal;
use crate::paths;
use crate::ports::db::FeatureRepository;
use crate::ports::step_executor::SyncOutcomeView;

use super::impl_traits::lock_registry;
use super::DagStepExecutor;

/// The row this feature's out-of-band syncs report through, created on the
/// first one and found on every one after.
///
/// A free function over the one port it needs, so what it does to the table is
/// assertable without an executor. `step_create` is a bare `INSERT`, so the
/// `step_get` is not an optimisation: without it the second attempt dies on the
/// primary key, and the id is derived rather than minted precisely so there is
/// a second attempt to find.
pub(crate) fn manual_sync_row(
    features: &dyn FeatureRepository,
    feature_id: &FeatureId,
    now: i64,
) -> Result<StepExecution, String> {
    let seeded = manual_sync_step_execution(feature_id, now);
    if let Some(existing) = features.step_get(&seeded.id)? {
        return Ok(existing);
    }
    features.step_create(seeded.clone())?;
    Ok(seeded)
}

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

        // A persisted row, because the id the turn streams against has to be one
        // the inspector can subscribe to — see `ResolveSyncContext::step_exec`.
        let step_exec = manual_sync_row(self.features.as_ref(), &fid, paths::now_ms())?;
        let writers = StatusWriters {
            features: self.features.as_ref(),
            notif: self.notif.as_ref(),
            f_id: &fid,
        };
        let start = Instant::now();
        let mut cost = 0.0_f64;
        let mut tokens = 0_i64;
        update_step_status(
            writers,
            &step_exec,
            StepTransition::running(0.0, Some(0), 0),
        );

        // Stop, for a turn no driver owns. Its own map rather than
        // `cancel_senders`: that one is keyed by feature id and owned by
        // `start_execution_with_ctx`, so a sync writing there would displace a
        // run's sender, and its entries are never removed — a stale `true`
        // would abort the next resolution before it started. This entry is
        // dropped when the turn ends.
        let (cancel_tx, cancel_rx) = watch::channel(false);
        lock_registry(&self.sync_cancels).insert(feature_id.to_string(), cancel_tx);

        let outcome = resolve_sync_conflicts(ResolveSyncContext {
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
            step_exec: &step_exec,
            thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
            agent_kind: &agent_kind,
            override_model: override_model.as_deref(),
            effort,
            max_budget_usd: budget::role_max_budget_usd(
                budget::base_max_budget_usd(
                    feature.max_budget_usd,
                    settings.default_max_budget_usd,
                ),
                budget::BUDGET_FRACTION_RESOLVER,
            ),
            cancel: Some(cancel_rx),
            spend: RunningSpend {
                cost: &mut cost,
                tokens: &mut tokens,
                start,
            },
            pricing: &self.pricing,
        })
        .await;

        lock_registry(&self.sync_cancels).remove(feature_id);

        let wall = start.elapsed().as_secs();
        match outcome {
            Ok(merge_commit_sha) => {
                update_step_status(
                    writers,
                    &step_exec,
                    StepTransition::completed(cost, tokens, wall, None, CacheTokens::default()),
                );
                Ok(SyncOutcomeView::Resolved { merge_commit_sha })
            }
            Err(failure) => {
                let stopped = matches!(failure, ResolveSyncError::Cancelled(_));
                let reason = failure.reason();
                let transition = if stopped {
                    StepTransition::interrupted(
                        cost,
                        tokens,
                        wall,
                        reason.clone(),
                        CacheTokens::default(),
                    )
                } else {
                    StepTransition::failed(
                        cost,
                        Some(tokens),
                        wall,
                        reason.clone(),
                        CacheTokens::default(),
                    )
                };
                update_step_status(writers, &step_exec, transition);
                Ok(SyncOutcomeView::ResolutionFailed {
                    reason,
                    conflict_files: list_unmerged_files(&*self.exec, &session.machine_id, worktree)
                        .await,
                })
            }
        }
    }
}
