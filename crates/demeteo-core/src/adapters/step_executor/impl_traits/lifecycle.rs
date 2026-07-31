use std::time::Instant;
use tokio::sync::watch;

use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::FeatureId;
use crate::domain::run_control::{shadow_refusal, RunAction};

use super::super::driver::ExecutionDriver;
use super::super::DagStepExecutor;
use super::execution_context::ExecutionContext;
use super::lock_registry;

impl DagStepExecutor {
    /// Resolve the execution context and start the driver loop.
    /// Used by [`replay_steps_from`](super::super::DagStepExecutor::replay_steps_from) which does not have a pre-resolved context.
    pub async fn start_execution_loop(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<(), String> {
        let ctx = self
            .resolve_execution_context(feature_id, project_id, workflow_id, description, false)
            .await?;
        self.start_execution_with_ctx(feature_id, ctx).await
    }

    /// Start the execution driver with a pre-resolved context.
    /// Avoids re-resolving the context (DB queries, path probe, etc.)
    /// when the caller already has one (e.g. [`feature_start`](crate::ports::step_executor::StepExecutor::feature_start)).
    pub async fn start_execution_with_ctx(
        &self,
        feature_id: &str,
        ctx: ExecutionContext,
    ) -> Result<(), String> {
        let f_id = FeatureId::from(feature_id.to_string());
        if self.driver_registry.is_live(&f_id) {
            // Already driving — refuse to start a second driver for the
            // same feature. Callers that want to retry should use
            // `replay_steps_from`, which cancels the old run first.
            return Ok(());
        }

        // The scheduling topology (P1.12), resolved on the context: either the
        // version's stored v2 document (V34, P3.6) or the migration of its step
        // list. Must build into a walkable graph — a failure here means corrupt
        // input, so refuse to start rather than spawn a driver that can never
        // schedule.
        let def_v2 = ctx.definition.clone();
        let graph =
            crate::domain::workflow_graph::WorkflowGraph::build(&def_v2).map_err(|findings| {
                format!(
                    "workflow graph is not schedulable: {}",
                    findings
                        .iter()
                        .map(|f| f.message.as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            })?;

        self.driver_registry.register(f_id.clone());

        let (cancel_tx, cancel_rx) = watch::channel(false);
        lock_registry(&self.cancel_senders).insert(feature_id.to_string(), cancel_tx);

        // Snapshot agent/model + loop-budget resolution inputs. Project
        // defaults come from the resolved settings; the per-run overrides
        // (feature-wide + per-step + loop budget) come off the Feature row.
        let default_agent_kind = ctx.settings.default_agent_kind.clone();
        let default_model = ctx.settings.default_model.clone();
        let default_effort = ctx.settings.default_effort;
        let project_default_loop_iterations = ctx.settings.default_loop_iterations;
        let project_default_max_budget_usd = ctx.settings.default_max_budget_usd;
        let feature_row = self.features.get(&f_id).ok().flatten();
        let feature_agent_kind = feature_row.as_ref().and_then(|f| f.agent_kind.clone());
        let feature_model = feature_row.as_ref().and_then(|f| f.model.clone());
        let feature_effort = feature_row.as_ref().and_then(|f| f.effort);
        let feature_model_for_budget = feature_model.clone();
        let loop_iterations_override = feature_row.as_ref().and_then(|f| f.loop_iterations);
        let max_budget_usd_override = feature_row.as_ref().and_then(|f| f.max_budget_usd);
        let step_overrides = feature_row
            .as_ref()
            .map(|f| f.step_overrides.clone())
            .unwrap_or_default();

        let driver = ExecutionDriver {
            features: self.features.clone(),
            gates: self.gates.clone(),
            projects: self.projects.clone(),
            signals: self.signals.clone(),
            notif: self.notif.clone(),
            notifications: self.notifications.clone(),
            registry: self.registry.clone(),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            artifacts: self.artifacts.clone(),
            attachments: self.attachments.clone(),
            app_settings: self.app_settings.clone(),
            git_ops: GitOpsHelper::new(self.app_settings.clone(), self.exec.clone()),
            merge_executor: self.merge_executor.clone(),
            subtask_runs: self.subtask_runs.clone(),
            sequence_resume: self.sequence_resume.clone(),
            mr_publisher: self.mr_publisher.clone(),
            gate_waiters: self.gate_waiters.clone(),
            driver_registry: self.driver_registry.clone(),
            pricing: self.pricing.clone(),
            f_id: f_id.clone(),
            f_id_str: feature_id.to_string(),
            machine_id_opt: ctx.machine_id_opt,
            target_dir: ctx.target_dir,
            branch_name: ctx.branch_name,
            base_ctx: ctx.base_ctx,
            steps: ctx.steps,
            def_v2,
            graph,
            start_time: Instant::now(),
            cancel_watch: cancel_rx,
            artifact_subdir: ctx.artifact_subdir,
            commit_artifacts: ctx.commit_artifacts,
            extra_writable_paths: ctx.settings.worktree_strategy.extra_writable_paths.clone(),
            feature_agent_kind,
            feature_model,
            feature_effort,
            step_overrides,
            default_agent_kind,
            default_model,
            default_effort,
            loop_iterations_override,
            project_default_loop_iterations,
            max_budget_usd_override,
            project_default_max_budget_usd,
            retry_ctx: None,
            resume_guard_done: false,
            current_model: feature_model_for_budget.clone(),
            context_budget_tokens: feature_model_for_budget
                .as_deref()
                .and_then(|m| self.pricing.context_window(m)),
            session_dirty: false,
            session_resume_summary: String::new(),
            session_cumulative_tokens: 0,
            last_cache_read: None,
            last_cache_creation: None,
            // Overwritten by `refresh_watchdog_budget` before the first
            // step dispatches; the bare feature id is a safe default.
            current_session_key: feature_id.to_string(),
        };

        let registry = self.driver_registry.clone();
        tokio::spawn(async move {
            // Own the deregister via a drop guard rather than a trailing
            // statement: if `driver.run()` panics, the panic unwinds the
            // task and a trailing `deregister` would never run, leaking the
            // `live` entry (`is_live` stays true forever, blocking
            // in-process recovery — see `deregister_guard`). The guard's
            // `Drop` fires on the panic unwind too.
            let _guard = registry.deregister_guard(f_id);
            driver.run().await;
        });

        Ok(())
    }

    /// Idempotently make sure a driver is running for `feature_id`. If
    /// one is already live, no-op. Otherwise re-resolve the context
    /// (replays, resumes, gate-decide-after-restart) and start one.
    ///
    /// This is the single recovery primitive used by `gate_decide`,
    /// `startup_watchdog`, and any future code path that needs a feature
    /// to make forward progress.
    pub async fn ensure_driver_running(&self, feature_id: &str) -> Result<(), String> {
        let f_id = FeatureId::from(feature_id.to_string());
        if self.driver_registry.is_live(&f_id) {
            return Ok(());
        }

        // A mirror-listed feature is a read-only shadow of a run a
        // `demeteo-runner` still owns (C4.2). Guarded here — not just at
        // the startup call sites — because this is the single recovery
        // primitive: a gate_decide or retry on a shadow (its mirrored
        // steps do sit in `awaiting_gate`) would otherwise arm a local
        // driver against a run another machine is driving.
        if self.runner_owned_features().contains(feature_id) {
            return Err(shadow_refusal(RunAction::Drive, feature_id));
        }

        let feature = self
            .features
            .get(&f_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;
        let workflow_id = feature
            .workflow_id
            .clone()
            .ok_or_else(|| format!("Feature '{}' has no workflow_id; cannot resume", feature_id))?;

        let ctx = self
            .resolve_execution_context(
                feature_id,
                &feature.project_id.0,
                workflow_id.as_str(),
                // The *description*, not the title: this is the same slot
                // `feature_start` fills with the rich prompt body, and it lands
                // in `{{feature_description}}` for every step the recovered
                // driver goes on to run (and in the memory-retrieval query).
                // Passing the short title here silently degraded every prompt
                // after a restart, gate decision, or watchdog recovery.
                &feature.description,
                false,
            )
            .await?;

        self.start_execution_with_ctx(feature_id, ctx).await
    }
}
