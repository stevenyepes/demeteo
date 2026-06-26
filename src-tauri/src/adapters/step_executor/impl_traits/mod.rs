use async_trait::async_trait;
use std::time::Instant;
use tokio::sync::watch;

use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::{FeatureId, GateDecisionId, StepExecutionId};
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::{GatePresenter, StepExecutor, SyncOutcomeView};

use self::execution_context::ExecutionContext;
use super::driver::ExecutionDriver;
use super::DagStepExecutor;

pub(crate) mod execution_context;
pub(crate) mod replay;

impl DagStepExecutor {
    /// Resolve the execution context and start the driver loop.
    /// Used by [`replay_steps_from`] which does not have a pre-resolved context.
    pub async fn start_execution_loop(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<(), String> {
        let ctx = self
            .resolve_execution_context(feature_id, project_id, workflow_id, description)
            .await?;
        self.start_execution_with_ctx(feature_id, ctx).await
    }

    /// Start the execution driver with a pre-resolved context.
    /// Avoids re-resolving the context (DB queries, path probe, etc.)
    /// when the caller already has one (e.g. [`feature_start`]).
    pub async fn start_execution_with_ctx(
        &self,
        feature_id: &str,
        ctx: ExecutionContext,
    ) -> Result<(), String> {
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancel_senders
            .lock()
            .unwrap()
            .insert(feature_id.to_string(), cancel_tx);

        // Snapshot agent/model + loop-budget resolution inputs. Project
        // defaults come from the resolved settings; the per-run overrides
        // (feature-wide + per-step + loop budget) come off the Feature row.
        let default_agent_kind = ctx.settings.default_agent_kind.clone();
        let default_model = ctx.settings.default_model.clone();
        let project_default_loop_iterations = ctx.settings.default_loop_iterations;
        let feature_row = self
            .features
            .get(&FeatureId::from(feature_id.to_string()))
            .ok()
            .flatten();
        let feature_agent_kind = feature_row.as_ref().and_then(|f| f.agent_kind.clone());
        let feature_model = feature_row.as_ref().and_then(|f| f.model.clone());
        let loop_iterations_override = feature_row.as_ref().and_then(|f| f.loop_iterations);
        let step_overrides = feature_row
            .as_ref()
            .map(|f| f.step_overrides.clone())
            .unwrap_or_default();

        let driver = ExecutionDriver {
            features: self.features.clone(),
            gates: self.gates.clone(),
            projects: self.projects.clone(),
            memory: self.memory.clone(),
            notif: self.notif.clone(),
            registry: self.registry.clone(),
            agent_exec: self.agent_exec.clone(),
            exec: self.exec.clone(),
            artifacts: self.artifacts.clone(),
            app_local_data_dir: self.app_local_data_dir.clone(),
            workspace_dir: self.workspace_dir.clone(),
            git_ops: GitOpsHelper::new(self.app_settings.clone(), self.exec.clone()),
            merge_executor: self.merge_executor.clone(),
            gate_senders: self.gate_senders.clone(),
            f_id: FeatureId::from(feature_id.to_string()),
            f_id_str: feature_id.to_string(),
            machine_id_opt: ctx.machine_id_opt,
            target_dir: ctx.target_dir,
            branch_name: ctx.branch_name,
            base_ctx: ctx.base_ctx,
            steps: ctx.steps,
            step_index: 0,
            start_time: Instant::now(),
            cancel_watch: cancel_rx,
            artifact_subdir: ctx.artifact_subdir,
            commit_artifacts: ctx.commit_artifacts,
            feature_agent_kind,
            feature_model,
            step_overrides,
            default_agent_kind,
            default_model,
            loop_iterations_override,
            project_default_loop_iterations,
            retry_ctx: None,
        };

        tokio::spawn(driver.run());

        Ok(())
    }
}

#[async_trait]
impl StepExecutor for DagStepExecutor {
    async fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
        commit_artifacts: Option<bool>,
        loop_iterations: Option<u32>,
        step_overrides: Vec<crate::domain::models::StepOverride>,
    ) -> Result<Feature, String> {
        if title.trim().is_empty() {
            return Err("Feature title cannot be empty.".to_string());
        }
        if description.trim().is_empty() {
            return Err("Feature description cannot be empty.".to_string());
        }

        let now = paths::now_ms();
        let feature_id = FeatureId::from(format!("f-{}", now));

        let ctx = self
            .resolve_execution_context(feature_id.as_str(), project_id, workflow_id, description)
            .await?;

        let git_ops = GitOpsHelper::new(self.app_settings.clone(), self.exec.clone());

        // Create the feature branch from whatever the local default_branch
        // points to. The branch refresh is done asynchronously below so
        // the user doesn't wait on a network fetch before seeing the
        // feature start.
        git_ops
            .create_feature_branch(
                ctx.machine_id_opt.as_deref(),
                &ctx.target_dir,
                &ctx.settings.worktree_strategy.default_branch,
                &ctx.branch_name,
            )
            .await?;

        // Refresh the local default_branch from origin asynchronously.
        // Best-effort: if the fetch fails (offline, no remote, auth) we
        // still proceed with whatever is local — the next sync will
        // catch up.
        {
            let exec = self.exec.clone();
            let app_settings = self.app_settings.clone();
            let machine_id = ctx.machine_id_opt.clone();
            let target_dir = ctx.target_dir.clone();
            let default_branch = ctx.settings.worktree_strategy.default_branch.clone();
            tokio::spawn(async move {
                let bg_git_ops = GitOpsHelper::new(app_settings, exec);
                let _ = bg_git_ops
                    .ensure_default_branch_updated(
                        machine_id.as_deref(),
                        &target_dir,
                        &default_branch,
                    )
                    .await;
            });
        }

        // Per-feature override of the project's commit_artifacts. None
        // means inherit; Some(true/false) is snapshotted on the Feature
        // row so project setting changes don't affect in-flight runs.
        let effective_commit = commit_artifacts.or(Some(ctx.commit_artifacts));

        let feature = Feature {
            id: feature_id.clone(),
            project_id: ctx.project_id.clone(),
            workflow_id: Some(ctx.workflow_id.clone()),
            title: title.to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: now,
            agent_kind: agent_kind.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            mr_url: None,
            mr_state: Some("none".to_string()),
            commit_artifacts: effective_commit,
            loop_iterations,
            step_overrides,
        };
        self.features.add(feature.clone())?;

        for (i, step) in ctx.steps.iter().enumerate() {
            let step_exec = StepExecution {
                id: StepExecutionId::from(format!("se-{}-{}", feature_id.as_str(), step.id.0)),
                feature_id: feature_id.clone(),
                step_id: step.id.clone(),
                step_index: i as u32,
                step_kind: step.kind.clone(),
                status: "pending".to_string(),
                cost_usd: Some(0.0),
                tokens: Some(0),
                wall_clock_secs: Some(0),
                artifact_path: None,
                artifact_paths: Vec::new(),
                error_message: None,
                iteration_count: 0,
                created_at: now,
                updated_at: now,
            };
            self.features.step_create(step_exec)?;
        }

        if let Err(e) = self
            .start_execution_with_ctx(feature_id.as_str(), ctx)
            .await
        {
            let _ = self.features.update(
                &feature_id,
                &FeaturePatch {
                    status: Some("failed".to_string()),
                    total_cost: None,
                    duration: None,
                    ..Default::default()
                },
            );
            let all_steps = self
                .features
                .steps_for_feature(&feature_id)
                .unwrap_or_default();
            for s in all_steps {
                let _ = self.features.step_update(
                    &s.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("failed".to_string()),
                        cost_usd: None,
                        tokens: None,
                        wall_clock_secs: None,
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: Some(Some(e.clone())),
                    },
                );
            }
            return Err(e);
        }

        Ok(feature)
    }

    async fn feature_pause(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn feature_resume(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    async fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
        if let Some(tx) = self.cancel_senders.lock().unwrap().get(feature_id) {
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
    ) -> Result<(), String> {
        let se_id = StepExecutionId::from(execution_id.to_string());
        let step_exec = self
            .features
            .step_get(&se_id)?
            .ok_or_else(|| format!("Step execution not found: {}", execution_id))?;

        if step_exec.status != "failed"
            && step_exec.status != "interrupted"
            && step_exec.status != "pending"
        {
            return Err(format!(
                "Cannot retry a step in '{}' status. Only failed or interrupted steps can be retried.",
                step_exec.status
            ));
        }

        self.replay_steps_from(execution_id, new_model, new_agent, true)
            .await
    }

    async fn replay_from_step(
        &self,
        execution_id: &str,
        new_model: Option<&str>,
        new_agent: Option<&str>,
    ) -> Result<(), String> {
        self.replay_steps_from(execution_id, new_model, new_agent, true)
            .await
    }

    async fn step_list_for_run(&self, feature_id: &str) -> Result<Vec<StepExecution>, String> {
        self.features
            .steps_for_feature(&FeatureId::from(feature_id.to_string()))
    }

    async fn feature_sync(
        &self,
        feature_id: &str,
        revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String> {
        self.feature_sync_impl(feature_id, revalidate_step_execution_id)
            .await
    }

    async fn feature_resolve_sync_conflicts(
        &self,
        feature_id: &str,
        conflict_files: &[String],
        revalidate_step_execution_id: Option<&str>,
    ) -> Result<SyncOutcomeView, String> {
        self.feature_resolve_sync_conflicts_impl(
            feature_id,
            conflict_files,
            revalidate_step_execution_id,
        )
        .await
    }
}

#[async_trait]
impl GatePresenter for DagStepExecutor {
    async fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String> {
        self.gates
            .pending_for_feature(&FeatureId::from(feature_id.to_string()))
    }

    async fn gate_decide(
        &self,
        step_execution_id: &str,
        decision: &str,
        feedback: Option<&str>,
    ) -> Result<(), String> {
        let se_id = StepExecutionId::from(step_execution_id.to_string());
        self.gates.decide(&se_id, decision, feedback)?;

        if let Some(tx) = self.gate_senders.lock().unwrap().remove(step_execution_id) {
            let gd = GateDecision {
                id: GateDecisionId::from(format!("gd-{}", step_execution_id)),
                step_execution_id: se_id,
                decision: Some(decision.to_string()),
                feedback: feedback.map(|s| s.to_string()),
                created_at: paths::now_ms(),
            };
            let _ = tx.send(gd);
        }
        Ok(())
    }
}

impl DagStepExecutor {
    pub fn startup_watchdog(&self) {
        if let Ok(projects) = self.projects.get_projects() {
            for p in projects {
                if let Ok(active) = self.features.get_active(&p.id) {
                    for f in active {
                        if f.status == "running" || f.status == "gated" {
                            let _ = self.projects.update_status(&p.id, "idle");
                            if let Ok(steps) = self.features.steps_for_feature(&f.id) {
                                for s in steps {
                                    if s.status == "running" || s.status == "awaiting_gate" {
                                        let was_awaiting = s.status == "awaiting_gate";
                                        let _ = self.features.step_update(
                                            &s.id,
                                            &StepExecutionPatch {
                                                status: Some("interrupted".to_string()),
                                                cost_usd: s.cost_usd.map(Some),
                                                wall_clock_secs: s.wall_clock_secs.map(Some),
                                                artifact_path: s
                                                    .artifact_path
                                                    .as_deref()
                                                    .map(|v| Some(v.to_string())),
                                                artifact_paths: Some(s.artifact_paths.clone()),
                                                error_message: Some(Some(if was_awaiting {
                                                    "Gate interrupted by system restart".to_string()
                                                } else {
                                                    "Step interrupted by system restart".to_string()
                                                })),
                                                ..Default::default()
                                            },
                                        );
                                        if !was_awaiting {
                                            let gate_dec_id =
                                                GateDecisionId::from(format!("gd-syn-{}", s.id.0));
                                            let gate_dec = GateDecision {
                                                id: gate_dec_id,
                                                step_execution_id: s.id.clone(),
                                                decision: None,
                                                feedback: None,
                                                created_at: paths::now_ms(),
                                            };
                                            let _ = self.gates.create(gate_dec);
                                        }
                                        let _ = self.notif.emit(&DomainEvent::GateRequired {
                                            feature_id: f.id.clone(),
                                            step_execution_id: s.id.clone(),
                                        });
                                    }
                                }
                                let _ = self.features.update(
                                    &f.id,
                                    &FeaturePatch {
                                        status: Some("awaiting_gate".to_string()),
                                        ..Default::default()
                                    },
                                );
                                let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
                                    feature_id: f.id.clone(),
                                    status: "awaiting_gate".into(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}
