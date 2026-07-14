use async_trait::async_trait;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;

use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::application::attachments::{commit_staged_attachments, StagedAttachmentInput};
use crate::domain::ids::{FeatureId, GateDecisionId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{Feature, GateDecision, StepExecution};
use crate::error::AppError;
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::{GatePresenter, StepExecutor, SyncOutcomeView};

use self::execution_context::ExecutionContext;
use super::driver::ExecutionDriver;
use super::DagStepExecutor;

pub(crate) mod execution_context;
pub(crate) mod replay;

/// Bootstrap phase vocabulary — `(id, label)`. Emitted as
/// [`DomainEvent::BootstrapProgress`] during [`StepExecutor::feature_start`]
/// so the UI can animate an inline stepper. The frontend renders `label`
/// verbatim and orders rows by `id`, so this list is the single source of
/// truth for the feature-start sub-steps (the runner adds its own
/// clone-phase ids in `demeteo-runner`). Phases fire `running` →
/// `completed`, or `failed` with the error in `detail`.
pub(crate) mod bootstrap_phase {
    pub const PREPARING: (&str, &str) = ("preparing", "Loading project & workflow");
    pub const CONNECTING: (&str, &str) = ("connecting", "Connecting to machine");
    pub const VERIFYING_REPO: (&str, &str) = ("verifying_repo", "Verifying repository");
    pub const PREPARING_CONTEXT: (&str, &str) = ("preparing_context", "Preparing context & memory");
    pub const SYNCING_ORIGIN: (&str, &str) = ("syncing_origin", "Syncing with origin");
    pub const CREATING_BRANCH: (&str, &str) = ("creating_branch", "Creating feature branch");
    pub const REGISTERING: (&str, &str) = ("registering", "Registering feature & steps");
    pub const STARTING_PIPELINE: (&str, &str) = ("starting_pipeline", "Starting pipeline");
}

impl DagStepExecutor {
    /// Emit a single [`DomainEvent::BootstrapProgress`]. Best-effort: a
    /// dropped progress event never blocks the bootstrap itself.
    pub(crate) fn emit_bootstrap(
        &self,
        feature_id: &str,
        phase: (&str, &str),
        status: &str,
        detail: Option<String>,
    ) {
        let _ = self.notif.emit(&DomainEvent::BootstrapProgress {
            feature_id: FeatureId::from(feature_id.to_string()),
            phase: phase.0.to_string(),
            label: phase.1.to_string(),
            status: status.to_string(),
            detail,
        });
    }

    /// The spawned tail of [`StepExecutor::feature_start`]. Runs the whole
    /// bootstrap and, on any failure, drives the feature to a terminal
    /// `failed` state. The phase that failed has already emitted a
    /// `BootstrapProgress { status: "failed" }` (in `resolve_execution_context`
    /// or the phase below), so here we only reconcile the durable state:
    /// mark the feature + any seeded steps failed and fire
    /// `FeatureStatusChanged` so the run list, the remote shadow, and the
    /// runner's `await_terminal_and_push` loop all observe the terminal state.
    async fn run_bootstrap_tail(
        &self,
        feature_id: FeatureId,
        project_id: String,
        workflow_id: String,
        description: String,
        staged_attachments: Vec<StagedAttachmentInput>,
    ) {
        if let Err(e) = self
            .run_bootstrap_tail_inner(
                &feature_id,
                &project_id,
                &workflow_id,
                &description,
                staged_attachments,
            )
            .await
        {
            let _ = self.features.update(
                &feature_id,
                &FeaturePatch {
                    status: Some("failed".to_string()),
                    ..Default::default()
                },
            );
            let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
                feature_id: feature_id.clone(),
                status: "failed".to_string(),
            });
            for s in self
                .features
                .steps_for_feature(&feature_id)
                .unwrap_or_default()
            {
                let _ = self.features.step_update(
                    &s.id,
                    &StepExecutionPatch {
                        status: Some("failed".to_string()),
                        error_message: Some(Some(e.clone())),
                        last_failure_fingerprint: None,
                        iteration_count: None,
                        cost_usd: None,
                        tokens: None,
                        wall_clock_secs: None,
                        artifact_path: None,
                        artifact_paths: None,
                        cache_read_input_tokens: None,
                        cache_creation_input_tokens: None,
                    },
                );
            }
            eprintln!(
                "[feature_start] bootstrap failed for {}: {}",
                feature_id.as_str(),
                e
            );
        }
    }

    async fn run_bootstrap_tail_inner(
        &self,
        feature_id: &FeatureId,
        project_id: &str,
        workflow_id: &str,
        description: &str,
        staged_attachments: Vec<StagedAttachmentInput>,
    ) -> Result<(), String> {
        let fid = feature_id.as_str();
        // Local aliases for the phases this fn owns (the first four are
        // emitted inside `resolve_execution_context`).
        let (sync, branch, register, start) = (
            bootstrap_phase::SYNCING_ORIGIN,
            bootstrap_phase::CREATING_BRANCH,
            bootstrap_phase::REGISTERING,
            bootstrap_phase::STARTING_PIPELINE,
        );

        // Phases 1-4: preparing / connecting / verifying_repo /
        // preparing_context (emitted from within the resolver).
        let ctx = self
            .resolve_execution_context(fid, project_id, workflow_id, description, true)
            .await?;

        let git_ops = GitOpsHelper::new(self.app_settings.clone(), self.exec.clone());
        let default_branch = ctx.settings.worktree_strategy.default_branch.clone();

        // Phase 5: refresh origin BEFORE cutting the branch. Awaited (unlike
        // the old fire-and-forget) but non-fatal — `create_feature_branch`
        // falls back to the local default if origin can't be reached, AND
        // the feature branch is always cut from `origin/<default>` when
        // available, so the pipeline proceeds either way. The error string
        // from `ensure_default_branch_updated` is already self-describing
        // (e.g. "local master is 71 commits behind origin/master but the
        // working tree has uncommitted changes; please `git pull` manually");
        // we surface it verbatim so the UI bootstrap detail tells the user
        // exactly what to do.
        self.emit_bootstrap(fid, sync, "running", None);
        let sync_detail = git_ops
            .ensure_default_branch_updated(
                ctx.machine_id_opt.as_deref(),
                &ctx.target_dir,
                &default_branch,
            )
            .await
            .err();
        self.emit_bootstrap(fid, sync, "completed", sync_detail);

        // Phase 6: cut the feature branch (from origin/<default>, else local).
        self.emit_bootstrap(fid, branch, "running", None);
        if let Err(e) = git_ops
            .create_feature_branch(
                ctx.machine_id_opt.as_deref(),
                &ctx.target_dir,
                &default_branch,
                &ctx.branch_name,
            )
            .await
        {
            self.emit_bootstrap(fid, branch, "failed", Some(e.clone()));
            return Err(e);
        }
        self.emit_bootstrap(fid, branch, "completed", None);

        // Phase 7: snapshot the resolved commit flag, seed the step rows, and
        // persist staged attachments before the driver reads them.
        self.emit_bootstrap(fid, register, "running", None);
        let _ = self.features.update(
            feature_id,
            &FeaturePatch {
                commit_artifacts: Some(Some(ctx.commit_artifacts)),
                ..Default::default()
            },
        );
        let now = paths::now_ms();
        for (i, step) in ctx.steps.iter().enumerate() {
            let step_exec = StepExecution {
                id: StepExecutionId::from(format!("se-{}-{}", fid, step.id.0)),
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
                cache_read_input_tokens: None,
                cache_creation_input_tokens: None,
                last_failure_fingerprint: None,
                created_at: now,
                updated_at: now,
            };
            self.features.step_create(step_exec)?;
        }
        if !staged_attachments.is_empty() {
            if let Err(e) = commit_staged_attachments(
                &self.features,
                &self.attachment_json,
                &self.attachments,
                fid,
                staged_attachments,
            ) {
                self.emit_bootstrap(fid, register, "failed", Some(e.to_string()));
                return Err(e.to_string());
            }
        }
        self.emit_bootstrap(fid, register, "completed", None);

        // Phase 8: flip the feature to "running" and start the driver. The
        // status flip + event let the run list / shadow leave "bootstrapping".
        self.emit_bootstrap(fid, start, "running", None);
        let _ = self.features.update(
            feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                ..Default::default()
            },
        );
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: feature_id.clone(),
            status: "running".to_string(),
        });
        if let Err(e) = self.start_execution_with_ctx(fid, ctx).await {
            self.emit_bootstrap(fid, start, "failed", Some(e.clone()));
            return Err(e);
        }
        self.emit_bootstrap(fid, start, "completed", None);
        Ok(())
    }
}

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
            .resolve_execution_context(feature_id, project_id, workflow_id, description, false)
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
        let f_id = FeatureId::from(feature_id.to_string());
        if self.driver_registry.is_live(&f_id) {
            // Already driving — refuse to start a second driver for the
            // same feature. Callers that want to retry should use
            // `replay_steps_from`, which cancels the old run first.
            return Ok(());
        }
        self.driver_registry.register(f_id.clone());

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
        let feature_row = self.features.get(&f_id).ok().flatten();
        let feature_agent_kind = feature_row.as_ref().and_then(|f| f.agent_kind.clone());
        let feature_model = feature_row.as_ref().and_then(|f| f.model.clone());
        let feature_model_for_budget = feature_model.clone();
        let loop_iterations_override = feature_row.as_ref().and_then(|f| f.loop_iterations);
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
            step_index: 0,
            start_time: Instant::now(),
            cancel_watch: cancel_rx,
            artifact_subdir: ctx.artifact_subdir,
            commit_artifacts: ctx.commit_artifacts,
            extra_writable_paths: ctx.settings.worktree_strategy.extra_writable_paths.clone(),
            feature_agent_kind,
            feature_model,
            step_overrides,
            default_agent_kind,
            default_model,
            loop_iterations_override,
            project_default_loop_iterations,
            retry_ctx: None,
            current_model: feature_model_for_budget.clone(),
            context_budget_tokens: feature_model_for_budget
                .as_deref()
                .and_then(|m| self.pricing.context_window(m)),
            session_dirty: false,
            session_resume_summary: String::new(),
            env_retried: std::collections::HashSet::new(),
            cached_plans: std::collections::HashMap::new(),
            sequence_checkpoints: std::collections::HashMap::new(),
            session_cumulative_tokens: 0,
            last_cache_read: None,
            last_cache_creation: None,
            // Overwritten by `refresh_watchdog_budget` before the first
            // step dispatches; the bare feature id is a safe default.
            current_session_key: feature_id.to_string(),
        };

        let registry = self.driver_registry.clone();
        tokio::spawn(async move {
            driver.run().await;
            registry.deregister(&f_id);
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
            return Err(format!(
                "Feature '{}' is a read-only shadow of a run owned by a demeteo-runner; \
                 this machine never drives it (decide its gates via the remote run instead)",
                feature_id
            ));
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

#[async_trait]
impl StepExecutor for DagStepExecutor {
    async fn feature_start(
        &self,
        feature_id: Option<String>,
        project_id: &str,
        workflow_id: &str,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
        commit_artifacts: Option<bool>,
        loop_iterations: Option<u32>,
        step_overrides: Vec<crate::domain::models::StepOverride>,
        staged_attachments: Vec<StagedAttachmentInput>,
    ) -> Result<Feature, String> {
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
        let feature = Feature {
            id: feature_id.clone(),
            project_id: ProjectId::from(project_id.to_string()),
            workflow_id: Some(WorkflowId::from(workflow_id.to_string())),
            title: title.to_string(),
            description: description.to_string(),
            status: "bootstrapping".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            tokens: 0,
            created_at: now,
            agent_kind: agent_kind.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            mr_url: None,
            mr_state: Some("none".to_string()),
            pr_title: None,
            pr_body: None,
            commit_artifacts,
            loop_iterations,
            step_overrides,
            attachments: Vec::new(),
        };
        self.features.add(feature.clone())?;

        // Spawn the bootstrap tail on a cheap clone (every field is an `Arc`).
        let this = self.clone();
        let fid = feature_id.clone();
        let project_id = project_id.to_string();
        let workflow_id = workflow_id.to_string();
        let description = description.to_string();
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
    ) -> Result<(), AppError> {
        let se_id = StepExecutionId::from(execution_id.to_string());
        let step_exec = self
            .features
            .step_get(&se_id)
            .map_err(AppError::from)?
            .ok_or_else(|| {
                AppError::not_found(format!("Step execution not found: {}", execution_id))
            })?;

        if step_exec.status != "failed"
            && step_exec.status != "interrupted"
            && step_exec.status != "pending"
        {
            return Err(AppError::validation(format!(
                "Cannot retry a step in '{}' status. Only failed or interrupted steps can be retried.",
                step_exec.status
            )));
        }

        self.assert_no_active_predecessors(&step_exec, "retrying this step")?;

        self.replay_steps_from(execution_id, new_model, new_agent, true)
            .await
            .map_err(AppError::from)
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
    ) -> Result<(), AppError> {
        let se_id = StepExecutionId::from(step_execution_id.to_string());

        // Pre-flight guard: refuse to apply a gate decision while an
        // earlier step is still running. The UI also disables the
        // Approve / Redirect buttons in this case, but the backend
        // must enforce the rule because a stale `gate_required` event
        // can race the agent's final artifact write and surface a
        // decidable gate while a predecessor is still in flight.
        let step_exec = self
            .features
            .step_get(&se_id)
            .map_err(AppError::from)?
            .ok_or_else(|| {
                AppError::not_found(format!("Step execution not found: {}", step_execution_id))
            })?;
        self.assert_no_active_predecessors(&step_exec, "deciding this gate")?;

        // 1. Durable: write the decision to the DB. UPSERT so the call
        //    is idempotent whether or not a row already exists. This is
        //    the source of truth — everything below is a wakeup hint.
        self.gates
            .upsert_decision(&se_id, decision, feedback, paths::now_ms())
            .map_err(AppError::from)?;

        let gd = GateDecision {
            id: GateDecisionId::from(format!("gd-{}", step_execution_id)),
            step_execution_id: se_id.clone(),
            decision: Some(decision.to_string()),
            feedback: feedback.map(|s| s.to_string()),
            created_at: paths::now_ms(),
        };

        // 2. Fast path: if the driver is alive and waiting on this
        //    step's waiter, deliver the decision in-memory. Missing
        //    waiter is *not* an error — the DB row will be picked up
        //    when the driver reconciles on its next startup.
        if let Some(waiter) = self
            .gate_waiters
            .lock()
            .unwrap()
            .get(step_execution_id)
            .cloned()
        {
            waiter.deliver(gd);
        }

        // 3. Self-healing: if the driver is dead (app restart, race,
        //    manual interruption), try to spawn one. The new driver
        //    will reconcile the decided gate on its first loop
        //    iteration. Best-effort: the decision is already durable
        //    in the DB, so a spawn failure (missing project, path
        //    probe failure, etc.) is logged but does NOT roll back
        //    the decision — the next legitimate operation will retry.
        if let Err(e) = self.ensure_driver_running(&step_exec.feature_id.0).await {
            eprintln!(
                "gate_decide: failed to ensure driver running for {}: {} \
                 (decision is durable; will retry on next operation)",
                step_exec.feature_id.0, e
            );
        }

        Ok(())
    }
}

impl DagStepExecutor {
    /// Refuse to act on `target` when an earlier step in the same feature
    /// is still non-terminal (`pending`, `running`, `verifying`, or
    /// `awaiting_gate`). Used by `step_retry` and `gate_decide` so a stale
    /// retry / approve click does not race a still-running predecessor.
    ///
    /// `intent` is the user-facing phrase that follows "before" in the
    /// returned message (e.g. "retrying this step", "deciding this gate").
    /// It is purely cosmetic so the two call sites can give the user a
    /// tailored sentence.
    ///
    /// Only `step_index < target.step_index` is considered — out-of-order
    /// races with later steps are out of scope (see Open Question #2 in
    /// `docs/RELIABILITY_PLAN.md`).
    pub(crate) fn assert_no_active_predecessors(
        &self,
        target: &StepExecution,
        intent: &str,
    ) -> Result<(), AppError> {
        let siblings = self
            .features
            .steps_for_feature(&target.feature_id)
            .map_err(AppError::from)?;
        for s in &siblings {
            if s.id == target.id {
                continue;
            }
            if s.step_index >= target.step_index {
                continue;
            }
            if matches!(
                s.status.as_str(),
                "pending" | "running" | "verifying" | "awaiting_gate"
            ) {
                return Err(AppError::validation(format!(
                    "Step '{}' is still {}; wait for it to finish before {}.",
                    s.step_id.0, s.status, intent
                )));
            }
        }
        Ok(())
    }

    /// Reconcile DB + notifications for any features that were left
    /// mid-run by a previous process. Synchronous (no driver spawns) so
    /// it can be called from the Tauri setup hook before the runtime
    /// hands control to user-driven tasks. Pair with
    /// [`resume_interrupted_features`] which spawns the actual drivers.
    ///
    /// Features present in the remote-run mirror (C4.2) are skipped:
    /// those rows are read-only *shadows* of features a `demeteo-runner`
    /// owns and is still driving on another machine. A shadow tracking a
    /// live remote run legitimately sits in `running`/`gated` across an
    /// app restart — no local process was ever driving it — so the
    /// watchdog must not mark its steps interrupted or re-emit gate
    /// prompts for it.
    pub fn startup_watchdog(&self) {
        let runner_owned = self.runner_owned_features();
        let Ok(projects) = self.projects.get_projects() else {
            return;
        };
        for p in &projects {
            if let Ok(active) = self.features.get_active(&p.id) {
                for f in active {
                    if runner_owned.contains(f.id.as_str()) {
                        continue;
                    }
                    if f.status == "running" || f.status == "gated" {
                        let _ = self.projects.update_status(&p.id, "idle");
                        if let Ok(steps) = self.features.steps_for_feature(&f.id) {
                            for s in steps {
                                if s.status == "running" || s.status == "awaiting_gate" {
                                    let was_awaiting = s.status == "awaiting_gate";
                                    // The step is being marked interrupted, so any
                                    // `running` subtask_runs row of its sequence
                                    // task loop is stale — close it, or the
                                    // dashboard's "nodes" count (which counts
                                    // running rows) over-reports forever.
                                    if let Err(e) = self
                                        .subtask_runs
                                        .subtask_runs_interrupt_stale(&s.id, paths::now_ms())
                                    {
                                        tracing::warn!(
                                            step_execution_id = %s.id.0,
                                            error = %e,
                                            "startup watchdog: could not close stale subtask_runs rows"
                                        );
                                    }
                                    let _ = self.features.step_update(
                                        &s.id,
                                        &StepExecutionPatch {
                                            last_failure_fingerprint: None,
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

        // Second pass: orphaned-pending reconciliation.
        // Hard-kills (OOM, crash, force-quit) leave step_executions with
        // status='pending' when the feature was already cancelled or failed.
        // These steps can never advance — mark them interrupted so the UI
        // shows a clean terminal state instead of a perpetual spinner.
        for p in &projects {
            if let Ok(all_features) = self.features.get_active(&p.id) {
                for f in all_features {
                    if runner_owned.contains(f.id.as_str()) {
                        // A shadow's pending steps mirror the runner's own
                        // rows mid-hydration — not orphans of a local crash.
                        continue;
                    }
                    if !matches!(f.status.as_str(), "cancelled" | "failed") {
                        continue;
                    }
                    if let Ok(steps) = self.features.steps_for_feature(&f.id) {
                        for s in steps.iter().filter(|s| s.status == "pending") {
                            let _ = self.features.step_update(
                                &s.id,
                                &StepExecutionPatch {
                                    last_failure_fingerprint: None,
                                    status: Some("interrupted".to_string()),
                                    error_message: Some(Some(
                                        "Step orphaned: feature ended before step ran".to_string(),
                                    )),
                                    ..Default::default()
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Resume every feature that [`startup_watchdog`] marked as
    /// `awaiting_gate`. Idempotent via [`DriverRegistry`]: if the
    /// runtime already has a driver alive for a feature, it's a no-op.
    ///
    /// Called once from the Tauri setup hook on a background task so
    /// that the gate prompts the watchdog re-emitted are actually
    /// backed by a live driver.
    ///
    /// Mirror-listed shadows are skipped (same rule as
    /// [`startup_watchdog`]): a shadow in `awaiting_gate`/`gated` is
    /// parked on the *runner*, not here — arming a local driver against
    /// it would have two engines driving one feature.
    pub async fn resume_interrupted_features(self: Arc<Self>) {
        let runner_owned = self.runner_owned_features();
        let Ok(projects) = self.projects.get_projects() else {
            return;
        };
        for p in projects {
            let Ok(active) = self.features.get_active(&p.id) else {
                continue;
            };
            for f in active {
                if runner_owned.contains(f.id.as_str()) {
                    continue;
                }
                if f.status == "awaiting_gate" || f.status == "gated" {
                    if let Err(e) = self.ensure_driver_running(&f.id.0).await {
                        eprintln!(
                            "resume_interrupted_features: failed to resume {}: {}",
                            f.id.0, e
                        );
                    }
                }
            }
        }
    }
}
