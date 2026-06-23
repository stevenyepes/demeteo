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
    pub(crate) async fn resolve_execution_context(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
    ) -> Result<ExecutionContext, String> {
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
        )
        .await?;

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
            &settings.artifact_subdir,
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
