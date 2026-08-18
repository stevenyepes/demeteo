use async_trait::async_trait;

use crate::domain::ids::{FeatureId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{Feature, StepExecution};
use crate::domain::run_control::{retry_refusal, shadow_refusal, RunAction};
use crate::error::AppError;
use crate::paths;
use crate::ports::step_executor::{FeatureLaunch, StepExecutor, SyncOutcomeView};

use super::super::DagStepExecutor;
use super::lock_registry;

#[async_trait]
impl StepExecutor for DagStepExecutor {
    async fn feature_start(&self, launch: FeatureLaunch) -> Result<Feature, String> {
        let FeatureLaunch {
            feature_id,
            project_id,
            workflow_id,
            title,
            description,
            agent_kind,
            model,
            effort,
            commit_artifacts,
            loop_iterations,
            max_budget_usd,
            step_overrides,
            staged_attachments,
            origin,
            diff_base_branch,
        } = launch;
        if title.trim().is_empty() {
            return Err("Feature title cannot be empty.".to_string());
        }
        if description.trim().is_empty() {
            return Err("Feature description cannot be empty.".to_string());
        }

        let now = paths::now_ms();
        // A caller-supplied id (the runner reusing `RunSpec::feature_id`)
        // wins so the laptop's eager shadow and the runner's own row
        // stay one feature; local launches keep the generated form.
        let feature_id = FeatureId::from(
            feature_id
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| format!("f-{}", now)),
        );

        // Insert the feature row eagerly with status "bootstrapping" and
        // return right away, so the caller is unblocked and the UI can
        // navigate straight into the feature detail view. The heavy,
        // possibly network-bound bootstrap (context resolve + SSH handshake +
        // origin sync + branch creation + step registration) then runs in the
        // spawned tail below, streaming `BootstrapProgress` events the UI
        // animates. On the desktop this replaces the old behavior where
        // `invoke('start_feature')` blocked on the whole bootstrap before
        // navigating; the runner's `await_terminal_and_push` already polls the
        // feature to a terminal status, so a bootstrap failure now surfaces
        // there as a "failed" feature (see `run_bootstrap_tail`).
        //
        // `commit_artifacts` is stored as the caller's *raw* override (which
        // may be `None` = inherit): `resolve_execution_context` reads it back
        // as the per-feature override, and the tail then snapshots the
        // resolved value onto the row so a later replay is stable.
        // Decision 38 (V33): resolve the workflow's latest version once,
        // now, and pin it on the row — every resume/replay reads the pin,
        // so a workflow edit can never change this run's graph. A missing
        // workflow pins nothing; the bootstrap tail then fails at resolve
        // exactly as before (and `resolve_pinned_version` backfills if a
        // version appears in between).
        let workflow_version_id = self
            .workflows
            .latest_version(&WorkflowId::from(workflow_id.clone()))
            .ok()
            .flatten()
            .map(|v| v.id);

        let feature = Feature {
            // The feature-wide run override (resolution tier 2). `None` =
            // inherit; the driver's chain bottoms out at `EffortLevel::DEFAULT`.
            // Per-step efforts ride inside `step_overrides`.
            effort,
            id: feature_id.clone(),
            project_id: ProjectId::from(project_id.clone()),
            workflow_id: Some(WorkflowId::from(workflow_id.clone())),
            workflow_version_id,
            title,
            description: description.clone(),
            status: "bootstrapping".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: now,
            agent_kind,
            model,
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts,
            loop_iterations,
            max_budget_usd,
            step_overrides,
            attachments: Vec::new(),
            harness_baseline: None,
            origin,
            diff_base_branch,
            resolved_branch: None,
        };
        self.features.add(feature.clone())?;

        // Spawn the bootstrap tail on a cheap clone (every field is an `Arc`).
        let this = self.clone();
        let fid = feature_id.clone();
        tokio::spawn(async move {
            this.run_bootstrap_tail(
                fid,
                project_id,
                workflow_id,
                description,
                staged_attachments,
            )
            .await;
        });

        Ok(feature)
    }

    async fn feature_pause(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn feature_resume(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
        // A shadow has no local driver to signal: the run is executing
        // under a `demeteo-runner` on another machine, which owns the
        // only cancel sender that can stop it (`cancel_run` RPC). Refuse
        // rather than find no sender in the map and report success — a
        // silent `Ok` here is a "Stop" the user watched do nothing.
        if self.runner_owned_features().contains(feature_id) {
            return Err(shadow_refusal(RunAction::Cancel, feature_id));
        }
        if let Some(tx) = lock_registry(&self.cancel_senders).get(feature_id) {
            let _ = tx.send(true);
        }
        // The out-of-band turns too: a manual sync resolution runs on a feature
        // whose run has already ended, so it never has a driver and its sender
        // is never in the map above.
        if let Some(tx) = lock_registry(&self.sync_cancels).get(feature_id) {
            let _ = tx.send(true);
        }
        Ok(())
    }

    async fn step_get(&self, execution_id: &str) -> Result<StepExecution, String> {
        self.features
            .step_get(&StepExecutionId::from(execution_id.to_string()))?
            .ok_or_else(|| "Step execution not found".to_string())
    }

    async fn step_retry(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
        new_effort: Option<crate::domain::models::EffortLevel>,
    ) -> Result<(), AppError> {
        let se_id = StepExecutionId::from(execution_id.to_string());
        let step_exec = self
            .features
            .step_get(&se_id)
            .map_err(AppError::from)?
            .ok_or_else(|| {
                AppError::not_found(format!("Step execution not found: {}", execution_id))
            })?;

        if let Some(refusal) = retry_refusal(&step_exec.status) {
            return Err(AppError::validation(refusal));
        }

        if let Some(refusal) =
            crate::domain::run_control::out_of_band_refusal(RunAction::Retry, &step_exec.step_id.0)
        {
            return Err(AppError::validation(refusal));
        }

        self.assert_no_active_predecessors(&step_exec, "retrying this step")?;

        // A shadow is not ours to replay. `replay_steps_from` rewinds the
        // step rows and calls `start_execution_loop` directly — it does not
        // route through `ensure_driver_running`, so its shadow guard never
        // fires and this machine would arm a *second* driver against a run
        // the runner is still driving, against a worktree that only exists
        // on the runner's box. There is no remote retry RPC yet, so the
        // honest answer is to refuse.
        if self
            .runner_owned_features()
            .contains(step_exec.feature_id.as_str())
        {
            return Err(AppError::validation(shadow_refusal(
                RunAction::Retry,
                &step_exec.feature_id.0,
            )));
        }

        // Keep any landed sequence prefix: a retry resumes from the task
        // that broke, which is the whole point of checkpointing it.
        self.replay_steps_from(execution_id, new_model, new_agent, new_effort, true, false)
            .await
            .map_err(AppError::from)
    }

    async fn replay_from_step(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
        new_effort: Option<crate::domain::models::EffortLevel>,
    ) -> Result<(), String> {
        // An explicit redo: drop any landed sequence prefix so the step runs
        // its whole task list, rather than silently skipping to the tail.
        self.replay_steps_from(execution_id, new_model, new_agent, new_effort, true, true)
            .await
    }

    async fn step_list_for_run(&self, feature_id: &str) -> Result<Vec<StepExecution>, String> {
        self.features
            .steps_for_feature(&FeatureId::from(feature_id.to_string()))
    }

    async fn feature_sync(&self, feature_id: &str) -> Result<SyncOutcomeView, String> {
        self.feature_sync_impl(feature_id).await
    }

    async fn feature_resolve_sync_conflicts(
        &self,
        feature_id: &str,
        conflict_files: &[String],
        asked: &crate::domain::sync_resolver::SyncResolverChoice,
    ) -> Result<SyncOutcomeView, String> {
        self.feature_resolve_sync_conflicts_impl(feature_id, conflict_files, asked)
            .await
    }

    async fn feature_sync_resolver(
        &self,
        feature_id: &str,
    ) -> Result<crate::ports::step_executor::SyncResolverView, String> {
        self.feature_sync_resolver_impl(feature_id)
    }
}
