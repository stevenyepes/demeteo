use std::time::Instant;

use tokio::sync::watch;

use crate::adapters::step_executor::setup::{
    build_base_ctx, fetch_default_settings, slug_from_description,
};
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::domain::ids::{FeatureId, GateDecisionId, ProjectId, StepExecutionId, WorkflowId};
use crate::domain::models::{Feature, GateDecision, StepConfig, StepExecution};
use crate::paths;
use crate::ports::db::{FeaturePatch, StepExecutionPatch};
use crate::ports::notification::DomainEvent;
use crate::ports::step_executor::{GatePresenter, StepExecutor};

use super::driver::ExecutionDriver;
use super::DagStepExecutor;

// ── Shared setup + spawn (called from feature_start and step_retry) ────────────

impl DagStepExecutor {
    pub fn start_execution_loop(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<(), String> {
        let project_id_typed = ProjectId::from(project_id.to_string());

        let settings = self
            .projects
            .get_settings(&project_id_typed)?
            .unwrap_or_else(fetch_default_settings);

        let all = self.projects.get_projects()?;
        let project = all
            .into_iter()
            .find(|p| p.id == project_id_typed)
            .ok_or_else(|| format!("Project not found: {}", project_id))?;

        let machine_id = if project.compute_type.to_lowercase() == "local" {
            None
        } else {
            project.remote_host.as_ref().map(|m| m.as_str())
        };

        let repos = self.projects.get_repositories_for(&project_id_typed)?;
        let repo = repos
            .first()
            .ok_or("No repository associated with this project.")?;
        let repo_path = repo.repo_path.clone();

        let target_dir = paths::repo_target_dir_str(
            &self.exec,
            &project.compute_type,
            project.remote_host.as_ref().map(|m| m.as_str()),
            project_id,
            &repo_path,
        )?;

        // Path probe
        let machine_id_for_check = if project.compute_type.to_lowercase() == "local" {
            "local".to_string()
        } else {
            project
                .remote_host
                .as_ref()
                .map(|m| m.as_str().to_string())
                .unwrap_or_else(|| "local".to_string())
        };
        let parent_dir = std::path::Path::new(&target_dir)
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        let probe = format!(
            "echo __DEMETEO_DIAG__ home=\\\"$HOME\\\" pwd=\\\"$PWD\\\"; \
             ls -la {} 2>&1; \
             test -d {} && echo __DEMETEO_DIAG__ exists || echo __DEMETEO_DIAG__ missing",
            paths::shell_escape_posix(&parent_dir),
            paths::shell_escape_posix(&target_dir),
        );
        let probe_output = self
            .exec
            .run_command(&machine_id_for_check, &probe)
            .unwrap_or_else(|e| format!("probe failed: {}", e));
        let path_ok = probe_output.contains("__DEMETEO_DIAG__ exists");
        if !path_ok {
            return Err(format!(
                "Repository target dir does not exist on '{}': {}\n\
                 Remote diagnostic probe output:\n{}\n\n\
                 If the parent dir listing is empty, the bootstrap clone \
                 did not actually run for this project — re-save the \
                 workspace settings to trigger a fresh bootstrap.",
                machine_id_for_check, target_dir, probe_output
            ));
        }

        let wf_id = WorkflowId::from(workflow_id.to_string());
        let latest_version = self
            .workflows
            .latest_version(&wf_id)?
            .ok_or_else(|| format!("No versions found for workflow: {}", workflow_id))?;

        let steps: Vec<StepConfig> = serde_json::from_str(&latest_version.steps_json)
            .map_err(|e| format!("Invalid workflow steps JSON: {}", e))?;

        if steps.is_empty() {
            return Err("Workflow has no steps.".to_string());
        }

        let slug = slug_from_description(description);
        let branch_name = format!("{}{}", settings.worktree_strategy.branch_prefix, feature_id);

        let machine_id_opt = machine_id.map(|s| s.to_string());

        // Set up cancel watch
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.cancel_senders
            .lock()
            .unwrap()
            .insert(feature_id.to_string(), cancel_tx);

        // Build base context
        let test_cmd = settings
            .worktree_strategy
            .test_command
            .clone()
            .unwrap_or_default();
        let build_cmd = settings
            .worktree_strategy
            .build_command
            .clone()
            .unwrap_or_default();
        let coverage_cmd = settings
            .worktree_strategy
            .coverage_command
            .clone()
            .unwrap_or_default();
        let conventions_content = settings
            .worktree_strategy
            .conventions_file
            .as_deref()
            .and_then(|path| self.exec.read_file(&machine_id_for_check, path).ok())
            .unwrap_or_default();
        let repo_list_str = repos
            .iter()
            .map(|r| r.repo_path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let memories = self
            .memory
            .memory_list(&project_id_typed, 100)
            .unwrap_or_default();
        let mut memory_md = String::new();
        for m in memories {
            let source_label = match m.source {
                crate::domain::memory::MemorySource::Agent => "Agent",
                crate::domain::memory::MemorySource::Human => "Human",
            };
            memory_md.push_str(&format!(
                "- **{}**: {} (Source: {})\n",
                m.key, m.value, source_label
            ));
        }

        let base_ctx = build_base_ctx(
            description,
            &slug,
            &branch_name,
            &repo_list_str,
            &test_cmd,
            &build_cmd,
            &coverage_cmd,
            &conventions_content,
            &memory_md,
        );

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
            git_ops: GitOpsHelper::new(self.app_settings.clone(), self.exec.clone()),
            gate_senders: self.gate_senders.clone(),
            f_id: FeatureId::from(feature_id.to_string()),
            f_id_str: feature_id.to_string(),
            machine_id_opt,
            target_dir,
            branch_name,
            base_ctx,
            steps,
            step_index: 0,
            start_time: Instant::now(),
            cancel_watch: cancel_rx,
        };

        tokio::spawn(driver.run());

        Ok(())
    }
}

// ── StepExecutor trait ─────────────────────────────────────────────────────────

impl StepExecutor for DagStepExecutor {
    fn feature_start(
        &self,
        project_id: &str,
        workflow_id: &str,
        title: &str,
        description: &str,
        agent_kind: Option<&str>,
        model: Option<&str>,
    ) -> Result<Feature, String> {
        if title.trim().is_empty() {
            return Err("Feature title cannot be empty.".to_string());
        }
        if description.trim().is_empty() {
            return Err("Feature description cannot be empty.".to_string());
        }

        let now = paths::now_ms();
        let feature_id = FeatureId::from(format!("f-{}", now));
        let project_id_typed = ProjectId::from(project_id.to_string());
        let workflow_id_typed = WorkflowId::from(workflow_id.to_string());

        let settings = self
            .projects
            .get_settings(&project_id_typed)?
            .unwrap_or_else(fetch_default_settings);

        let all = self.projects.get_projects()?;
        let project = all
            .into_iter()
            .find(|p| p.id == project_id_typed)
            .ok_or_else(|| format!("Project not found: {}", project_id))?;

        let repos = self.projects.get_repositories_for(&project_id_typed)?;
        let repo = repos
            .first()
            .ok_or("No repository associated with this project.")?;
        let repo_path = repo.repo_path.clone();

        let _target_dir = paths::repo_target_dir_str(
            &self.exec,
            &project.compute_type,
            project.remote_host.as_ref().map(|m| m.as_str()),
            project_id,
            &repo_path,
        )?;

        let latest_version = self
            .workflows
            .latest_version(&workflow_id_typed)?
            .ok_or_else(|| format!("No versions found for workflow: {}", workflow_id))?;

        let steps: Vec<StepConfig> = serde_json::from_str(&latest_version.steps_json)
            .map_err(|e| format!("Invalid workflow steps JSON: {}", e))?;

        if steps.is_empty() {
            return Err("Workflow has no steps.".to_string());
        }

        let branch_name = format!(
            "{}{}",
            settings.worktree_strategy.branch_prefix,
            feature_id.as_str()
        );

        let machine_id = if project.compute_type.to_lowercase() == "local" {
            None
        } else {
            project.remote_host.as_ref().map(|m| m.as_str())
        };
        let machine_id_opt = machine_id.map(|s| s.to_string());

        let git_ops = GitOpsHelper::new(self.app_settings.clone(), self.exec.clone());
        git_ops.create_feature_branch(
            machine_id_opt.as_deref(),
            &_target_dir,
            &settings.worktree_strategy.default_branch,
            &branch_name,
        )?;

        let feature = Feature {
            id: feature_id.clone(),
            project_id: project_id_typed.clone(),
            workflow_id: Some(workflow_id_typed.clone()),
            title: title.to_string(),
            status: "running".to_string(),
            total_cost: 0.0,
            duration: "0s".to_string(),
            created_at: now,
            agent_kind: agent_kind.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            mr_url: None,
            mr_state: Some("none".to_string()),
        };
        self.features.add(feature.clone())?;

        for (i, step) in steps.iter().enumerate() {
            let step_exec = StepExecution {
                id: StepExecutionId::from(format!("se-{}-{}", feature_id.as_str(), step.id.0)),
                feature_id: feature_id.clone(),
                step_id: step.id.clone(),
                step_index: i as u32,
                step_kind: step.kind.clone(),
                status: "pending".to_string(),
                cost_usd: Some(0.0),
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

        if let Err(e) =
            self.start_execution_loop(feature_id.as_str(), project_id, workflow_id, description)
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

    fn feature_pause(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn feature_resume(&self, _feature_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn feature_cancel(&self, feature_id: &str) -> Result<(), String> {
        if let Some(tx) = self.cancel_senders.lock().unwrap().get(feature_id) {
            let _ = tx.send(true);
        }
        Ok(())
    }

    fn step_get(&self, execution_id: &str) -> Result<StepExecution, String> {
        self.features
            .step_get(&StepExecutionId::from(execution_id.to_string()))?
            .ok_or_else(|| "Step execution not found".to_string())
    }

    fn step_retry(&self, execution_id: &str, new_model: Option<&str>) -> Result<(), String> {
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

        let feature_id = &step_exec.feature_id;
        let feature = self
            .features
            .get(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        if let Some(model) = new_model {
            self.features.update(
                feature_id,
                &FeaturePatch {
                    model: Some(Some(model.to_string())),
                    ..Default::default()
                },
            )?;
        }

        let mut workflow_id = feature.workflow_id.clone();

        if workflow_id.is_none() {
            let step_execs = self.features.steps_for_feature(feature_id)?;
            let step_ids: Vec<String> = step_execs.iter().map(|s| s.step_id.0.clone()).collect();

            let workflows = self.workflows.list()?;
            for w in workflows {
                if let Some(version) = self.workflows.latest_version(&w.id)? {
                    if let Ok(steps) = serde_json::from_str::<Vec<StepConfig>>(&version.steps_json)
                    {
                        let w_step_ids: Vec<String> =
                            steps.iter().map(|s| s.id.0.clone()).collect();
                        if w_step_ids == step_ids {
                            self.features.update_workflow_id(feature_id, &w.id)?;
                            workflow_id = Some(w.id);
                            break;
                        }
                    }
                }
            }
        }

        let workflow_id = workflow_id.ok_or_else(|| {
            format!(
                "Workflow ID not found for feature {}. \
                 This legacy feature does not match any current workflow steps.",
                feature_id
            )
        })?;

        let all_steps = self.features.steps_for_feature(feature_id)?;
        let mut patch_list: Vec<(StepExecutionId, String)> = Vec::new();
        for s in &all_steps {
            if s.step_index >= step_exec.step_index {
                patch_list.push((s.id.clone(), s.status.clone()));
                self.features.step_update(
                    &s.id,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some("pending".to_string()),
                        cost_usd: s.cost_usd.map(Some),
                        wall_clock_secs: s.wall_clock_secs.map(Some),
                        artifact_path: None,
                        artifact_paths: Some(Vec::new()),
                        error_message: Some(None),
                    },
                )?;
            }
        }

        let prev_feature_status = feature.status.clone();
        self.features.update(
            feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                total_cost: None,
                duration: None,
                ..Default::default()
            },
        )?;
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: feature_id.clone(),
            status: "running".into(),
        });

        if let Err(e) = self.start_execution_loop(
            feature_id.as_str(),
            &feature.project_id.0,
            workflow_id.as_str(),
            &feature.title,
        ) {
            for (sid, original_status) in &patch_list {
                let _ = self.features.step_update(
                    sid,
                    &StepExecutionPatch {
                        iteration_count: None,
                        status: Some(original_status.clone()),
                        cost_usd: None,
                        wall_clock_secs: None,
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: None,
                    },
                );
            }
            let _ = self.features.update(
                feature_id,
                &FeaturePatch {
                    status: Some(prev_feature_status.clone()),
                    total_cost: None,
                    duration: None,
                    ..Default::default()
                },
            );
            return Err(e);
        }

        Ok(())
    }

    fn replay_from_step(&self, execution_id: &str, new_model: Option<&str>) -> Result<(), String> {
        let se_id = StepExecutionId::from(execution_id.to_string());
        let step_exec = self
            .features
            .step_get(&se_id)?
            .ok_or_else(|| format!("Step execution not found: {}", execution_id))?;

        let feature_id = &step_exec.feature_id;
        let feature = self
            .features
            .get(feature_id)?
            .ok_or_else(|| format!("Feature not found: {}", feature_id))?;

        // Cancel any in-flight execution and force-kill the old session
        // so get_or_spawn creates a fresh one instead of returning the
        // stale session (which would deadlock the transport on prompt).
        if feature.status == "running" {
            self.feature_cancel(feature_id.as_str())?;
            let reg = self.registry.clone();
            let fid = feature_id.to_string();
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    reg.kill(&fid).await;
                })
            });
            // Yield to let the old driver's cancel handler finish
            // writing its terminal state before we overwrite it.
            std::thread::sleep(std::time::Duration::from_millis(300));
        }

        if let Some(model) = new_model {
            self.features.update(
                feature_id,
                &FeaturePatch {
                    model: Some(Some(model.to_string())),
                    ..Default::default()
                },
            )?;
        }

        // Resolve workflow (same as step_retry)
        let mut workflow_id = feature.workflow_id.clone();
        if workflow_id.is_none() {
            let step_execs = self.features.steps_for_feature(feature_id)?;
            let step_ids: Vec<String> = step_execs.iter().map(|s| s.step_id.0.clone()).collect();
            let workflows = self.workflows.list()?;
            for w in workflows {
                if let Some(version) = self.workflows.latest_version(&w.id)? {
                    if let Ok(steps) = serde_json::from_str::<Vec<StepConfig>>(&version.steps_json)
                    {
                        let w_step_ids: Vec<String> =
                            steps.iter().map(|s| s.id.0.clone()).collect();
                        if w_step_ids == step_ids {
                            self.features.update_workflow_id(feature_id, &w.id)?;
                            workflow_id = Some(w.id);
                            break;
                        }
                    }
                }
            }
        }
        let workflow_id = workflow_id.ok_or_else(|| {
            format!(
                "Workflow ID not found for feature {}. \
                 This legacy feature does not match any current workflow steps.",
                feature_id
            )
        })?;

        // Reset target step and all downstream steps to pending
        let all_steps = self.features.steps_for_feature(feature_id)?;
        let mut patch_list: Vec<(StepExecutionId, String)> = Vec::new();
        for s in &all_steps {
            if s.step_index >= step_exec.step_index {
                patch_list.push((s.id.clone(), s.status.clone()));
                self.features.step_update(
                    &s.id,
                    &StepExecutionPatch {
                        status: Some("pending".to_string()),
                        cost_usd: s.cost_usd.map(Some),
                        wall_clock_secs: s.wall_clock_secs.map(Some),
                        artifact_path: None,
                        artifact_paths: Some(Vec::new()),
                        error_message: Some(None),
                        ..Default::default()
                    },
                )?;
                // Clear gate decisions for gate steps in the affected range
                if s.step_kind == "gate" {
                    let _ = self.gates.reset_for_step_execution(&s.id);
                }
            }
        }

        let prev_feature_status = feature.status.clone();
        self.features.update(
            feature_id,
            &FeaturePatch {
                status: Some("running".to_string()),
                total_cost: None,
                duration: None,
                ..Default::default()
            },
        )?;
        let _ = self.notif.emit(&DomainEvent::FeatureStatusChanged {
            feature_id: feature_id.clone(),
            status: "running".into(),
        });

        if let Err(e) = self.start_execution_loop(
            feature_id.as_str(),
            &feature.project_id.0,
            workflow_id.as_str(),
            &feature.title,
        ) {
            for (sid, original_status) in &patch_list {
                let _ = self.features.step_update(
                    sid,
                    &StepExecutionPatch {
                        status: Some(original_status.clone()),
                        cost_usd: None,
                        wall_clock_secs: None,
                        artifact_path: None,
                        artifact_paths: None,
                        error_message: None,
                        ..Default::default()
                    },
                );
            }
            let _ = self.features.update(
                feature_id,
                &FeaturePatch {
                    status: Some(prev_feature_status.clone()),
                    total_cost: None,
                    duration: None,
                    ..Default::default()
                },
            );
            return Err(e);
        }

        Ok(())
    }

    fn step_list_for_run(&self, feature_id: &str) -> Result<Vec<StepExecution>, String> {
        self.features
            .steps_for_feature(&FeatureId::from(feature_id.to_string()))
    }
}

// ── GatePresenter trait ────────────────────────────────────────────────────────

impl GatePresenter for DagStepExecutor {
    fn gate_pending_for_run(&self, feature_id: &str) -> Result<Option<GateDecision>, String> {
        self.gates
            .pending_for_feature(&FeatureId::from(feature_id.to_string()))
    }

    fn gate_decide(
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

// ── Startup watchdog ───────────────────────────────────────────────────────────

impl DagStepExecutor {
    /// On every app launch, find any steps that were left mid-flight
    /// (`status = "running"`) by a previous process and surface a
    /// *synthetic gate* for each. This is the Q14 "Resume / Restart /
    /// Skip" affordance — without it, the user would either lose
    /// progress or accidentally double-bill a step on relaunch.
    ///
    /// For each interrupted step:
    ///   1. Mark it `interrupted` and attach a clear error message.
    ///   2. Insert a `GateDecision` row so the existing `gate_decide`
    ///      Tauri command can record the user's choice.
    ///   3. Emit `GateRequired` so the existing `GateView` UI pops up.
    ///   4. Park the feature itself at `awaiting_gate` so the next
    ///      `feature_resume` (after the user clicks "Resume") knows
    ///      which step to drive forward.
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
                                        // Only emit a synthetic gate for non-gate
                                        // steps. A gate step that was already
                                        // awaiting a decision has a row in
                                        // `gate_decisions` already — just emit
                                        // the event so the UI re-shows it.
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
                                // Park the feature so the user knows it's
                                // waiting on them. The next feature_resume
                                // call (after they click "Resume" in the
                                // gate) is what drives the loop forward.
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
