//! Feature-branch sync with the upstream `base_branch`.
//!
//! Two Tauri commands surface this code path:
//!
//! - `feature_sync`: merges `origin/<base>` into the feature branch.
//!   A clean merge is a `SyncOutcomeView::Ok`; unmerged paths are a
//!   `SyncOutcomeView::Conflict`, which is the only outcome the UI offers
//!   "Resolve with agent" for. Everything that failed before the merge is a
//!   `SyncOutcomeView::Blocked` — [`crate::domain::sync_failure`] owns that
//!   split and `command_view` is the only thing that decides it.
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
    continue_sync_resolution, resolve_sync_conflicts, ContinueSyncContext, ResolveSyncContext,
    ResolveSyncError, SYNC_RESOLVER_THREAD_PREFIX,
};
use crate::domain::agent_session::budget;
use crate::domain::ids::FeatureId;
use crate::domain::models::StepExecution;
use crate::domain::step_seed::manual_sync_step_execution;
use crate::domain::sync_resolver::SyncResolverChoice;
use crate::domain::sync_session::{
    intervention_refusal, sync_liveness, SyncIntervention, SyncStanding,
};
use crate::paths;
use crate::ports::db::FeatureRepository;
use crate::ports::step_executor::SyncOutcomeView;

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

/// What a sync of this project must prove before it publishes.
///
/// Beside [`sync_base`] and for the same reason: the "Sync with main" button
/// and the workflow's own `sync` node both have to answer it, and two
/// derivations are two chances for one project's syncs to gate on different
/// commands — the shape `diff_base::resolve` was centralised to prevent for the
/// base branch.
pub(crate) fn sync_gate(
    settings: &crate::domain::models::ProjectSettings,
) -> crate::ports::worktree_ops::MergeGate<'_> {
    crate::ports::worktree_ops::MergeGate {
        prepare: settings.worktree_strategy.prepare_command.as_deref(),
        harness: settings.worktree_strategy.test_command.as_deref(),
    }
}

impl DagStepExecutor {
    /// Hold the in-flight entry a resolution would claim, so the refusal that
    /// serialises two of them is assertable without racing two real turns.
    ///
    /// The guard is handed back because dropping it releases the slot: bound to
    /// `_` the claim would be gone before the call under test made it.
    #[cfg(test)]
    pub(crate) fn claim_sync_cancel_for_test(
        &self,
        feature_id: &str,
        tx: watch::Sender<bool>,
    ) -> crate::application::sync_turns::SyncTurn<'_> {
        self.sync_turns
            .claim(feature_id, Some(tx))
            .expect("the fixture's registry starts empty")
    }

    /// Tauri entry point for the "Sync with main" command. Resolves
    /// the feature branch + project state, asks the merge executor to
    /// do the actual git work, and translates the result into a
    /// `SyncOutcomeView` for the UI.
    pub(crate) async fn feature_sync_impl(
        &self,
        feature_id: &str,
    ) -> Result<SyncOutcomeView, String> {
        let (fid, feature, settings) = self.sync_context(feature_id)?;
        let base_branch = sync_base(&feature, &settings)?;
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        // The slot this merge needs is claimed inside
        // `sync_feature_with_upstream`, which is the one frame every entry
        // point — this command, and the workflow's own `sync` node — passes
        // through, and the frame the destructive worktree sweep lives under.
        // Claimed here instead, the node reached that sweep holding nothing.
        crate::domain::sync_failure::command_view(
            self.merge_executor
                .sync_feature_with_upstream(
                    &fid,
                    &feature_branch,
                    &base_branch,
                    sync_gate(&settings),
                )
                .await,
        )
    }

    /// Tauri entry point for the two presses a sync blocked on a divergence
    /// offers.
    ///
    /// The same three lines `feature_sync_impl` resolves its branches with,
    /// because this *is* that sync: the reconcile runs in the same worktree,
    /// ahead of the same base merge, under the same gate. Nothing here judges
    /// whether the press is still a safe one — that is re-measured against git
    /// where the move is made
    /// ([`divergence_move`](crate::domain::upstream_feature::divergence_move)),
    /// which is the only place the answer cannot already be stale.
    pub(crate) async fn feature_reconcile_impl(
        &self,
        feature_id: &str,
        reconcile: crate::domain::upstream_feature::DivergenceReconcile,
    ) -> Result<Option<crate::ports::sync_session::SyncSessionView>, String> {
        let (fid, feature, settings) = self.sync_context(feature_id)?;
        let base_branch = sync_base(&feature, &settings)?;
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        self.merge_executor
            .reconcile_feature_with_origin(
                &fid,
                &feature_branch,
                &base_branch,
                sync_gate(&settings),
                reconcile,
            )
            .await
    }

    /// Tauri entry point for the read behind those presses.
    ///
    /// No base branch: a divergence is a statement about the feature branch and
    /// its own upstream, so a run whose base cannot be resolved still gets a
    /// true answer about the half this question is over.
    pub(crate) async fn feature_divergence_impl(
        &self,
        feature_id: &str,
    ) -> Result<Option<crate::domain::models::FeatureDivergence>, String> {
        let (fid, feature, settings) = self.sync_context(feature_id)?;
        self.merge_executor
            .feature_divergence(
                &fid,
                &feature.run_branch(&settings.worktree_strategy.branch_prefix),
            )
            .await
    }

    /// The feature row and the project settings every sync-shaped command
    /// resolves its branches from, with the project default standing in for a
    /// project that has none.
    fn sync_context(
        &self,
        feature_id: &str,
    ) -> Result<
        (
            FeatureId,
            crate::domain::models::Feature,
            crate::domain::models::ProjectSettings,
        ),
        String,
    > {
        let fid = FeatureId::from(feature_id.to_string());
        let feature = self
            .features
            .get(&fid)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;
        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);
        Ok((fid, feature, settings))
    }

    /// Tauri entry point for the staleness signal on the run header.
    ///
    /// The same three lines `feature_sync_impl` resolves its branches with, and
    /// deliberately so: a drift chip counted against a base the sync would not
    /// have merged is a number about a branch nobody is going to touch.
    pub(crate) async fn feature_drift_impl(
        &self,
        feature_id: &str,
        refresh: bool,
    ) -> Result<crate::domain::models::FeatureDrift, String> {
        let (fid, feature, settings) = self.sync_context(feature_id)?;
        let base_branch = sync_base(&feature, &settings)?;
        let feature_branch = feature.run_branch(&settings.worktree_strategy.branch_prefix);

        self.merge_executor
            .feature_drift(&fid, &feature_branch, &base_branch, refresh)
            .await
    }

    /// Who the "Resolve with agent" button would spawn if nobody asked for
    /// anything — the same two rows and the same chain, read without running a
    /// turn.
    pub(crate) fn feature_sync_resolver_impl(
        &self,
        feature_id: &str,
    ) -> Result<crate::ports::step_executor::SyncResolverView, String> {
        let (_, feature, settings) = self.sync_context(feature_id)?;
        Ok(crate::domain::sync_resolver::resolve_stored(
            &SyncResolverChoice::default(),
            &feature,
            &settings,
        )
        .into())
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
        asked: &SyncResolverChoice,
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
        if let Some(refusal) = intervention_refusal(
            SyncIntervention::Resolve,
            SyncStanding {
                status: session.status,
                published: session.pushed_at.is_some(),
                blocked_stage: session.blocked_stage,
                feature_status: &feature.status,
                liveness: sync_liveness(self.sync_turns.claimed(feature_id), &feature.status),
            },
        ) {
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
        let chosen = crate::domain::sync_resolver::resolve_stored(asked, &feature, &settings);

        // Stop, for a turn no driver owns. Its own map rather than
        // `cancel_senders`: that one is keyed by feature id and owned by
        // `start_execution_with_ctx`, so a sync writing there would displace a
        // run's sender, and its entries are never removed — a stale `true`
        // would abort the next resolution before it started. This entry is
        // dropped when the turn ends.
        //
        // Claimed before anything else, and under one guard, because the entry
        // is also the only thing serialising two resolutions of one feature.
        // `reconcile` rewrites a `resolving` row back to `conflicted` whenever
        // the merge is still open — sound for a row whose writer is gone, and
        // false in-process while this very turn holds it — so a second window's
        // click passes `intervention_refusal` and does the thing
        // `user_may_intervene` exists to prevent. It would also displace this
        // turn's sender and swallow its Stop.
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let Some(_turn) = self.sync_turns.claim(feature_id, Some(cancel_tx)) else {
            return Err(
                "A resolution is already running for this feature; wait for it or stop it."
                    .to_string(),
            );
        };

        // A persisted row, because the id the turn streams against has to be one
        // the inspector can subscribe to — see `ResolveSyncContext::step_exec`.
        let step_exec = manual_sync_row(self.features.as_ref(), &fid, paths::now_ms())?;
        let writers = StatusWriters {
            features: self.features.as_ref(),
            notif: self.notif.as_ref(),
            f_id: &fid,
        };
        let start = Instant::now();
        // Seeded from the row rather than from zero: the row is reused by every
        // out-of-band sync this feature runs, and the header's spend is a sum
        // over rows, so restarting the count makes the previous attempt's
        // dollars vanish from the feature's total. The run loop carries a
        // re-dispatched node's spend forward for the same reason
        // (`StepTransition::running`).
        let mut cost = step_exec.cost_usd.unwrap_or(0.0);
        let mut tokens = step_exec.tokens.unwrap_or(0);
        update_step_status(
            writers,
            &step_exec,
            StepTransition::running(cost, Some(tokens), 0),
        );

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
            declared_conflicts: &session.conflict_files,
            gate: sync_gate(&settings),
            step_exec: &step_exec,
            thread_id_prefix: SYNC_RESOLVER_THREAD_PREFIX,
            agent_kind: &chosen.agent_kind,
            override_model: chosen.model.as_deref(),
            effort: chosen.effort,
            max_budget_usd: budget::role_max_budget_usd(
                budget::base_max_budget_usd(
                    feature.max_budget_usd,
                    settings.default_max_budget_usd,
                ),
                budget::BUDGET_FRACTION_RESOLVER,
            ),
            review_before_push: settings.sync_review_before_push,
            feature_status: &feature.status,
            cancel: Some(cancel_rx),
            spend: RunningSpend {
                cost: &mut cost,
                tokens: &mut tokens,
                start,
            },
            pricing: &self.pricing,
        })
        .await;

        let wall = start.elapsed().as_secs();
        match outcome {
            Ok(resolved) => {
                update_step_status(
                    writers,
                    &step_exec,
                    StepTransition::completed(cost, tokens, wall, None, resolved.cache),
                );
                Ok(SyncOutcomeView::Resolved {
                    merge_commit_sha: resolved.merge_commit_sha,
                })
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

    /// Take a conflict the user resolved themselves: verify the tree, run the
    /// project's checks in it, commit and publish.
    ///
    /// The same operation as `feature_resolve_sync_conflicts_impl` with the
    /// agent turn removed, and it goes through every one of the same guards for
    /// the same reasons — the claim so it cannot run beside a resolution, the
    /// refusal so it cannot touch a worktree a run owns, the row so the
    /// timeline shows what happened. It reuses `SyncIntervention::Resolve`
    /// because the precondition is identical: an unpublished conflict on a
    /// feature nothing else is driving.
    pub(crate) async fn feature_continue_sync_impl(
        &self,
        feature_id: &str,
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
                "This feature has no sync to finish. Run 'Sync with main' first.".to_string()
            })?;
        if let Some(refusal) = intervention_refusal(
            SyncIntervention::Resolve,
            SyncStanding {
                status: session.status,
                published: session.pushed_at.is_some(),
                blocked_stage: session.blocked_stage,
                feature_status: &feature.status,
                liveness: sync_liveness(self.sync_turns.claimed(feature_id), &feature.status),
            },
        ) {
            return Err(refusal.to_string());
        }
        let worktree = session.worktree_path.as_deref().ok_or_else(|| {
            "This sync never provisioned a worktree, so there is nothing to finish in one."
                .to_string()
        })?;

        let settings = self
            .projects
            .get_settings(&feature.project_id)?
            .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);

        let (cancel_tx, cancel_rx) = watch::channel(false);
        let Some(_turn) = self.sync_turns.claim(feature_id, Some(cancel_tx)) else {
            return Err(
                "A resolution is already running for this feature; wait for it or stop it."
                    .to_string(),
            );
        };

        let step_exec = manual_sync_row(self.features.as_ref(), &fid, paths::now_ms())?;
        let writers = StatusWriters {
            features: self.features.as_ref(),
            notif: self.notif.as_ref(),
            f_id: &fid,
        };
        let start = Instant::now();
        let cost = step_exec.cost_usd.unwrap_or(0.0);
        let tokens = step_exec.tokens.unwrap_or(0);
        update_step_status(
            writers,
            &step_exec,
            StepTransition::running(cost, Some(tokens), 0),
        );

        let outcome = continue_sync_resolution(ContinueSyncContext {
            exec: &self.exec,
            app_settings: &self.app_settings,
            git_ops: &self.git_ops,
            merge_executor: &self.merge_executor,
            feature_id: &fid,
            repo_dir: &session.repo_dir,
            resolved_cwd: worktree,
            machine_str: &session.machine_id,
            feature_branch: &session.feature_branch,
            base_branch: &session.base_branch,
            declared_conflicts: &session.conflict_files,
            gate: sync_gate(&settings),
            review_before_push: settings.sync_review_before_push,
            feature_status: &feature.status,
            cancel: Some(cancel_rx),
        })
        .await;

        let wall = start.elapsed().as_secs();
        match outcome {
            Ok(resolved) => {
                update_step_status(
                    writers,
                    &step_exec,
                    StepTransition::completed(cost, tokens, wall, None, resolved.cache),
                );
                Ok(SyncOutcomeView::Resolved {
                    merge_commit_sha: resolved.merge_commit_sha,
                })
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
