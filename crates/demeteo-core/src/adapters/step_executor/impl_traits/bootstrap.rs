use crate::adapters::step_executor::preflight::PREFLIGHT_PROBE_TIMEOUT_S;
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::application::attachments::{commit_staged_attachments, StagedAttachmentInput};
use crate::domain::ids::FeatureId;
use crate::domain::step_seed::seed_step_executions;
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;

use super::super::DagStepExecutor;
use super::bootstrap_phase;
use super::execution_context::ExecutionContext;

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

    /// The spawned tail of
    /// [`StepExecutor::feature_start`](crate::ports::step_executor::StepExecutor::feature_start).
    /// Runs the whole bootstrap and, on any failure, drives the feature to a
    /// terminal `failed` state. The phase that failed has already emitted a
    /// `BootstrapProgress { status: "failed" }` (in `resolve_execution_context`
    /// or the phase below), so here we only reconcile the durable state:
    /// mark the feature + any seeded steps failed and fire
    /// `FeatureStatusChanged` so the run list, the remote shadow, and the
    /// runner's `await_terminal_and_push` loop all observe the terminal state.
    pub(super) async fn run_bootstrap_tail(
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
        let (sync, branch, start) = (
            bootstrap_phase::SYNCING_ORIGIN,
            bootstrap_phase::CREATING_BRANCH,
            bootstrap_phase::STARTING_PIPELINE,
        );

        // Phases 1-4: preparing / connecting / verifying_repo /
        // preparing_context (emitted from within the resolver).
        let ctx = self
            .resolve_execution_context(fid, project_id, workflow_id, description, true)
            .await?;

        self.run_harness_preflight(fid, &ctx).await?;

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

        self.register_steps(feature_id, &ctx, staged_attachments)?;

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

    /// Harness preflight (HB1/HB4). Resolve the binaries every one of the
    /// project's configured commands names — `prepare_command`,
    /// `test_command`, and each named harness — on the machine that will run
    /// them, before a single token is spent.
    ///
    /// Here rather than in a graph node because it has to hold for *any*
    /// workflow, including one the user drew with no baseline node in it. A
    /// node protects only the graphs containing it.
    ///
    /// Before `registering` on purpose: a blocking verdict returns from this
    /// function, and `run_bootstrap_tail` then marks the feature failed. With
    /// no step rows seeded yet there is nothing half-registered to reconcile
    /// — the run simply never started, which is the honest description.
    ///
    /// Probes only. `prepare_command` is probed but never *run* here, and
    /// neither is the suite: running belongs to the `baseline-harness` node
    /// at the head of the graph, which reaches them at the same point in the
    /// timeline without charging every launch a minute of wall-clock before
    /// anything visible happens.
    async fn run_harness_preflight(&self, fid: &str, ctx: &ExecutionContext) -> Result<(), String> {
        let preflight = bootstrap_phase::HARNESS_PREFLIGHT;
        self.emit_bootstrap(fid, preflight, "running", None);
        crate::adapters::step_executor::preflight::validate_bootstrap_tools(
            self.exec.as_ref(),
            ctx.machine_id_opt
                .as_deref()
                .unwrap_or(crate::domain::ids::LOCAL_MACHINE),
            &ctx.target_dir,
            std::time::Duration::from_secs(PREFLIGHT_PROBE_TIMEOUT_S),
        )
        .await
        .map_err(|error| {
            self.emit_bootstrap(fid, preflight, "failed", Some(error.clone()));
            error
        })?;
        let verdict = crate::adapters::step_executor::preflight::probe_configured_commands(
            self.exec.as_ref(),
            ctx.machine_id_opt
                .as_deref()
                .unwrap_or(crate::domain::ids::LOCAL_MACHINE),
            &ctx.target_dir,
            &ctx.settings.worktree_strategy,
            std::time::Duration::from_secs(PREFLIGHT_PROBE_TIMEOUT_S),
        )
        .await;
        self.emit_bootstrap(fid, preflight, verdict.phase_status(), verdict.detail());
        match verdict.launch_refusal() {
            Some(refusal) => Err(refusal),
            None => Ok(()),
        }
    }

    /// Snapshot the resolved commit flag, seed the step rows, and persist
    /// staged attachments before the driver reads them.
    fn register_steps(
        &self,
        feature_id: &FeatureId,
        ctx: &ExecutionContext,
        staged_attachments: Vec<StagedAttachmentInput>,
    ) -> Result<(), String> {
        let fid = feature_id.as_str();
        let register = bootstrap_phase::REGISTERING;

        self.emit_bootstrap(fid, register, "running", None);
        let _ = self.features.update(
            feature_id,
            &FeaturePatch {
                commit_artifacts: Some(Some(ctx.commit_artifacts)),
                ..Default::default()
            },
        );
        for step_exec in seed_step_executions(feature_id, &ctx.steps, paths::now_ms()) {
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
        Ok(())
    }
}
