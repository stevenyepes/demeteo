use super::super::DagStepExecutor;
use crate::adapters::step_executor::setup::{
    build_base_ctx, fetch_default_settings, slug_from_description,
};
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::models::workflow_v2::definition_matches_steps;
use crate::domain::models::{ProjectSettings, StepConfig};
use crate::domain::prompt_context::PromptContext;
use crate::domain::workflow_overrides::{bake_step_overrides, overlay_workflow_defaults};
use crate::paths;

pub(crate) mod repo_probe;

pub struct ExecutionContext {
    pub project_id: ProjectId,
    pub workflow_id: WorkflowId,
    pub settings: ProjectSettings,
    pub target_dir: String,
    pub branch_name: String,
    pub steps: Vec<StepConfig>,
    /// The pinned version's **schema-v2 definition** — the run's scheduling
    /// topology (P1.12), taken from the stored document when the version has
    /// one (V34, P3.6) and migrated from `steps` when it doesn't.
    ///
    /// Carried beside `steps` rather than derived from it because a graph the
    /// builder authored has edges a chain projection cannot reproduce: read a
    /// diamond back through `steps` alone and the engine would silently run it
    /// as a line. `steps` stays the *config* source every node handler reads;
    /// this owns the edges, joins, and per-class retry.
    pub definition: crate::domain::models::workflow_v2::WorkflowDefinitionV2,
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
    /// Resolve the workflow version this feature runs (decision 38,
    /// V33): the pinned row when the feature carries one; otherwise
    /// latest, which is then pinned so every later resume of this
    /// feature reads the same graph. Pre-V33 features (and rows whose
    /// eager `feature_start` pin failed) take the backfill path exactly
    /// once — after that, editing the workflow mid-run can never change
    /// the running graph.
    pub(crate) fn resolve_pinned_version(
        &self,
        feature_id: &str,
        wf_id: &WorkflowId,
    ) -> Result<crate::domain::models::WorkflowVersion, String> {
        let fid = FeatureId::from(feature_id.to_string());
        let pinned = self
            .features
            .get(&fid)
            .ok()
            .flatten()
            .and_then(|f| f.workflow_version_id);
        if let Some(vid) = pinned {
            return match self.workflows.version_get(&vid) {
                Ok(Some(v)) => Ok(v),
                // Versions are immutable rows that outlive their pin; a
                // miss means a hand-edited DB, and silently falling back
                // to latest would run a graph the user never launched.
                Ok(None) => Err(format!(
                    "Pinned workflow version not found: {} (workflow {})",
                    vid.0, wf_id.0
                )),
                Err(e) => Err(e),
            };
        }
        let latest = self
            .workflows
            .latest_version(wf_id)?
            .ok_or_else(|| format!("No versions found for workflow: {}", wf_id.0))?;
        // Best-effort backfill: a failed pin write only means the next
        // resume resolves latest again (the pre-V33 behavior).
        let _ = self.features.pin_workflow_version(&fid, &latest.id);
        Ok(latest)
    }
    /// Build the `project_memory` markdown injected into agent prompts.
    ///
    /// When the memory agent is configured, retrieves the semantically most
    /// relevant memories for this feature — cosine similarity of the embedded
    /// `query` (feature description) × the memory's confidence — and records
    /// their use. Falls back to the legacy confidence/recency ordering when the
    /// agent is disabled or embedding fails, so prompts always get memory.
    pub(crate) async fn build_memory_md(&self, project_id: &ProjectId, query: &str) -> String {
        use crate::domain::memory::{rank_memories, render_memory_md, ProjectMemoryEntry};

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
                    rank_memories(&memories, &q, config.min_confidence, config.top_k)
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

        render_memory_md(&selected)
    }

    /// Resolve the context for a run. When `emit_bootstrap` is set (only the
    /// initial [`feature_start`] tail passes `true`; replay / resume / gate
    /// recovery pass `false`), the network- and DB-bound sub-steps stream
    /// `DomainEvent::BootstrapProgress` so the UI can animate an inline
    /// stepper. See [`super::bootstrap_phase`] for the phase vocabulary.
    pub(crate) async fn resolve_execution_context(
        &self,
        feature_id: &str,
        project_id: &str,
        workflow_id: &str,
        description: &str,
        emit_bootstrap: bool,
    ) -> Result<ExecutionContext, String> {
        use super::bootstrap_phase as bp;
        // Convenience: only emit when this is a fresh feature start.
        let emit = |phase: (&str, &str), status: &str, detail: Option<String>| {
            if emit_bootstrap {
                self.emit_bootstrap(feature_id, phase, status, detail);
            }
        };

        emit(bp::PREPARING, "running", None);
        let project_id_typed = ProjectId::from(project_id.to_string());
        let mut settings = self
            .projects
            .get_settings(&project_id_typed)?
            .unwrap_or_else(fetch_default_settings);

        let all = self.projects.get_projects()?;
        let project = match all.into_iter().find(|p| p.id == project_id_typed) {
            Some(p) => p,
            None => {
                let e = format!("Project not found: {}", project_id);
                emit(bp::PREPARING, "failed", Some(e.clone()));
                return Err(e);
            }
        };

        let machine_id = if project.compute_type.to_lowercase() == "local" {
            None
        } else {
            project.remote_host.as_ref().map(|m| m.as_str())
        };

        let repos = self.projects.get_repositories_for(&project_id_typed)?;
        let repo = match repos.first() {
            Some(r) => r,
            None => {
                let e = "No repository associated with this project.".to_string();
                emit(bp::PREPARING, "failed", Some(e.clone()));
                return Err(e);
            }
        };
        let repo_path = repo.repo_path.clone();
        emit(bp::PREPARING, "completed", None);

        let is_remote = project.compute_type.to_lowercase() != "local";
        let target_dir = if !is_remote {
            paths::repo_target_dir_local(&self.workspace_dir, project_id, &repo_path)
                .to_string_lossy()
                .to_string()
        } else {
            // First remote contact — the SSH session is established lazily
            // here, so this is where the handshake latency lands.
            emit(bp::CONNECTING, "running", None);
            match paths::repo_target_dir_str(
                &self.exec,
                &project.compute_type,
                project.remote_host.as_ref().map(|m| m.as_str()),
                project_id,
                &repo_path,
                None,
            )
            .await
            {
                Ok(dir) => {
                    emit(bp::CONNECTING, "completed", None);
                    dir
                }
                Err(e) => {
                    emit(bp::CONNECTING, "failed", Some(e.clone()));
                    return Err(e);
                }
            }
        };

        let wf_id = WorkflowId::from(workflow_id.to_string());

        // Project-scoped overrides for this workflow (V14/V15), split into the
        // workflow-level row (applies to all steps) and per-step rows.
        let project_overrides = self
            .projects
            .list_overrides_for_workflow(&project_id_typed, &wf_id)
            .unwrap_or_default();

        overlay_workflow_defaults(&mut settings, &project_overrides);

        // Decision 38 (V33): the run path reads the feature's *pinned*
        // version — never a re-resolved latest — so a mid-run workflow
        // edit cannot change the graph under the driver.
        let version = match self.resolve_pinned_version(feature_id, &wf_id) {
            Ok(v) => v,
            Err(e) => {
                emit(bp::PREPARING, "failed", Some(e.clone()));
                return Err(e);
            }
        };

        let mut steps: Vec<StepConfig> = match serde_json::from_str(&version.steps_json) {
            Ok(s) => s,
            Err(e) => {
                let e = format!("Invalid workflow steps JSON: {}", e);
                emit(bp::PREPARING, "failed", Some(e.clone()));
                return Err(e);
            }
        };

        // The run's topology. Prefers the version's stored v2 document (V34,
        // P3.6) so a graph authored in the builder runs the edges it was
        // drawn with; falls back to migrating `steps` for every pre-P3.6 row.
        // `definition_matches_steps` owns why that fallback exists.
        let definition = {
            let stored = version.definition(wf_id.as_str());
            if definition_matches_steps(&stored, &steps) {
                stored
            } else {
                tracing::warn!(
                    version_id = %version.id,
                    "stored v2 definition does not match the version's step list; \
                     scheduling from the migrated step list instead"
                );
                crate::domain::models::workflow_migrate::migrate_v1_to_v2(
                    wf_id.clone(),
                    wf_id.as_str(),
                    &steps,
                )
            }
        };

        if steps.is_empty() {
            let e = "Workflow has no steps.".to_string();
            emit(bp::PREPARING, "failed", Some(e.clone()));
            return Err(e);
        }

        bake_step_overrides(&mut steps, &project_overrides);

        let slug = slug_from_description(description);
        let branch_name = format!("{}{}", settings.worktree_strategy.branch_prefix, feature_id);

        let machine_id_opt = machine_id.map(|s| s.to_string());
        let machine_id_for_check = machine_id_opt
            .clone()
            .unwrap_or_else(|| crate::domain::ids::LOCAL_MACHINE.to_string());

        emit(bp::VERIFYING_REPO, "running", None);
        if let Err(e) =
            repo_probe::verify_repo_present(self.exec.as_ref(), &machine_id_for_check, &target_dir)
                .await
        {
            emit(bp::VERIFYING_REPO, "failed", Some(e.clone()));
            return Err(e);
        }
        emit(bp::VERIFYING_REPO, "completed", None);

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
        emit(bp::PREPARING_CONTEXT, "running", None);
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

        emit(bp::PREPARING_CONTEXT, "completed", None);

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
            definition,
            base_ctx,
            machine_id_opt,
            artifact_subdir,
            commit_artifacts,
        })
    }
}
