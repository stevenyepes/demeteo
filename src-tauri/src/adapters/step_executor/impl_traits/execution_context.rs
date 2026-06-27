use super::super::DagStepExecutor;
use crate::adapters::step_executor::setup::{
    build_base_ctx, fetch_default_settings, slug_from_description,
};
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::models::{ProjectSettings, StepConfig};
use crate::domain::prompt_context::PromptContext;
use crate::paths;

pub struct ExecutionContext {
    pub project_id: ProjectId,
    pub workflow_id: WorkflowId,
    pub settings: ProjectSettings,
    pub target_dir: String,
    pub branch_name: String,
    pub steps: Vec<StepConfig>,
    pub base_ctx: PromptContext,
    pub machine_id_opt: Option<String>,
    /// Repo-relative folder under the worktree root where agents
    /// write their reports. Resolved from `ProjectSettings` at
    /// feature-start time and snapshotted on the `Feature` row so
    /// later changes to project settings don't affect in-flight
    /// features. Default: `"artifacts/"`. See migration V12.
    pub artifact_subdir: String,
    /// Whether to include the artifact subdir when the orchestrator
    /// runs `commit_worktree_changes` for this feature. `true` →
    /// reports land in the PR. `false` → reports stay in demeteo's
    /// `FsArtifactStore` only. Resolved as:
    /// `features.commit_artifacts ?? settings.commit_artifacts`.
    pub commit_artifacts: bool,
}

impl DagStepExecutor {
    /// Build the `project_memory` markdown injected into agent prompts.
    ///
    /// When the memory agent is configured, retrieves the semantically most
    /// relevant memories for this feature — cosine similarity of the embedded
    /// `query` (feature description) × the memory's confidence — and records
    /// their use. Falls back to the legacy confidence/recency ordering when the
    /// agent is disabled or embedding fails, so prompts always get memory.
    pub(crate) async fn build_memory_md(&self, project_id: &ProjectId, query: &str) -> String {
        use crate::domain::memory::{cosine_similarity, MemorySource, ProjectMemoryEntry};

        let memories = self.memory.memory_list(project_id, 200).unwrap_or_default();
        if memories.is_empty() {
            return String::new();
        }
        let config = crate::application::memory::load_config(self.app_settings.as_ref());

        let selected: Vec<&ProjectMemoryEntry> = if config.is_usable() && !query.trim().is_empty() {
            let api_key = crate::application::memory::load_api_key();
            match self
                .memory_llm
                .embed(
                    config.embed_endpoint_or_chat(),
                    &config.embed_model,
                    api_key.as_deref(),
                    vec![query.to_string()],
                )
                .await
            {
                Ok(mut vecs) if !vecs.is_empty() => {
                    let q = vecs.remove(0);
                    let mut scored: Vec<(&ProjectMemoryEntry, f32)> = memories
                        .iter()
                        .filter(|m| m.confidence >= config.min_confidence)
                        .filter_map(|m| {
                            m.embedding
                                .as_ref()
                                .map(|e| (m, cosine_similarity(&q, e) * m.confidence as f32))
                        })
                        .collect();
                    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
                    scored
                        .into_iter()
                        .take(config.top_k)
                        .map(|(m, _)| m)
                        .collect()
                }
                _ => memories.iter().take(20).collect(),
            }
        } else {
            memories.iter().take(20).collect()
        };

        let used_ids: Vec<String> = selected.iter().map(|m| m.id.clone()).collect();
        let _ = self
            .memory
            .memory_mark_used(&used_ids, crate::paths::now_ms());

        let mut md = String::new();
        for m in selected {
            let source_label = match m.source {
                MemorySource::Agent => "Agent",
                MemorySource::Human => "Human",
            };
            let body = m.statement.as_deref().unwrap_or(&m.value);
            match m.memory_type {
                Some(t) => md.push_str(&format!(
                    "- [{}] {} (Source: {})\n",
                    t.as_str(),
                    body,
                    source_label
                )),
                None => md.push_str(&format!(
                    "- **{}**: {} (Source: {})\n",
                    m.key, body, source_label
                )),
            }
        }
        md
    }

    pub(crate) async fn resolve_execution_context(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<ExecutionContext, String> {
        let project_id_typed = ProjectId::from(project_id.to_string());
        let mut settings = self
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

        let target_dir = if project.compute_type.to_lowercase() == "local" {
            paths::repo_target_dir_local(&self.workspace_dir, project_id, &repo_path)
                .to_string_lossy()
                .to_string()
        } else {
            paths::repo_target_dir_str(
                &self.exec,
                &project.compute_type,
                project.remote_host.as_ref().map(|m| m.as_str()),
                project_id,
                &repo_path,
                None,
            )
            .await?
        };

        let wf_id = WorkflowId::from(workflow_id.to_string());

        // Project-scoped overrides for this workflow (V14/V15), split into the
        // workflow-level row (applies to all steps) and per-step rows.
        let project_overrides = self
            .projects
            .list_overrides_for_workflow(&project_id_typed, &wf_id)
            .unwrap_or_default();

        // Workflow-level override overlays the project defaults for THIS
        // workflow only. This keeps `resolve_agent_model` untouched — it just
        // becomes the effective `default_agent_kind` / `default_model`, so a
        // more specific intent (step agent/model, feature-wide run override,
        // per-step run override) still wins.
        if let Some(wf_level) = project_overrides.iter().find(|o| o.step_id.is_none()) {
            if wf_level.agent_kind.is_some() {
                settings.default_agent_kind = wf_level.agent_kind.clone();
            }
            if wf_level.model.is_some() {
                settings.default_model = wf_level.model.clone();
            }
        }

        let latest_version = self
            .workflows
            .latest_version(&wf_id)?
            .ok_or_else(|| format!("No versions found for workflow: {}", workflow_id))?;

        let mut steps: Vec<StepConfig> = serde_json::from_str(&latest_version.steps_json)
            .map_err(|e| format!("Invalid workflow steps JSON: {}", e))?;

        if steps.is_empty() {
            return Err("Workflow has no steps.".to_string());
        }

        // Bake step-level project overrides onto the matching steps. Each field
        // overlays independently, replacing the workflow author's value. This
        // sits at the workflow-step tier of `resolve_agent_model`, so it beats
        // the author's choice but still loses to a run-time launch override.
        for ov in project_overrides.iter() {
            let Some(step_id) = ov.step_id.as_deref() else {
                continue;
            };
            if let Some(step) = steps.iter_mut().find(|s| s.id.0 == step_id) {
                if ov.agent_kind.is_some() {
                    step.agent_kind = ov.agent_kind.clone();
                }
                if ov.model.is_some() {
                    step.model = ov.model.clone();
                }
            }
        }

        let slug = slug_from_description(description);
        let branch_name = format!("{}{}", settings.worktree_strategy.branch_prefix, feature_id);

        let machine_id_opt = machine_id.map(|s| s.to_string());
        let machine_id_for_check = machine_id_opt
            .clone()
            .unwrap_or_else(|| "local".to_string());

        // Path probe
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
            .await
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
        let conventions_content =
            if let Some(path) = settings.worktree_strategy.conventions_file.as_deref() {
                let exec = self.exec.clone();
                let path = path.to_string();
                let machine = machine_id_for_check.clone();
                exec.read_file(&machine, &path).await.unwrap_or_default()
            } else {
                String::new()
            };
        let repo_list_str = repos
            .iter()
            .map(|r| r.repo_path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let memory_md = self.build_memory_md(&project_id_typed, description).await;

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
            &settings.artifact_subdir,
            // First turn of the feature → no recap needed. The
            // watchdog populates this on subsequent turns when
            // it resets the session.
            "",
        );

        // Snapshot the artifact subdir + commit flag from project
        // settings, then honour the Feature row's per-feature override
        // if one is already in the DB (replay / re-entry path).
        let artifact_subdir = settings.artifact_subdir.clone();
        let mut commit_artifacts = settings.commit_artifacts;
        if let Ok(Some(existing)) = self.features.get(&FeatureId::from(feature_id.to_string())) {
            if let Some(override_flag) = existing.commit_artifacts {
                commit_artifacts = override_flag;
            }
        }

        Ok(ExecutionContext {
            project_id: project_id_typed,
            workflow_id: wf_id,
            settings,
            target_dir,
            branch_name,
            steps,
            base_ctx,
            machine_id_opt,
            artifact_subdir,
            commit_artifacts,
        })
    }
}
