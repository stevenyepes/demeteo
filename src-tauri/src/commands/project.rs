use crate::application::projects::{ProjectConfig, RepoDirtyStatus};
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, RepoHealthStatus, Repository};
use crate::error::AppError;
use crate::paths;
use crate::state::AppContext;
use tauri::State;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ProjectCreateResponse {
    pub id: String,
    pub success: bool,
}

#[tauri::command]
pub async fn create_project(
    ctx: State<'_, AppContext>,
    config: ProjectConfig,
) -> Result<ProjectCreateResponse, AppError> {
    let now = paths::now_ms();
    let id_str = format!("p{}", now);
    let id = ProjectId::from(id_str.clone());

    let project = Project {
        id: id.clone(),
        name: config.name.clone(),
        compute_type: config.compute_type.clone(),
        remote_host: config.remote_host.clone().map(MachineId::from),
        status: "bootstrapping".to_string(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: now,
    };

    ctx.projects.add(project)?;

    for (i, repo_cfg) in config.repos.iter().enumerate() {
        let repo_id = RepositoryId::from(format!("{}_r{}", id_str, i));
        let repo = Repository {
            id: repo_id,
            project_id: id.clone(),
            provider_id: ProviderId::from(repo_cfg.provider_id.clone()),
            repo_path: repo_cfg.repo_path.clone(),
        };
        ctx.projects.add_repository(repo)?;
    }

    Ok(ProjectCreateResponse {
        id: id_str,
        success: true,
    })
}

#[tauri::command]
pub fn get_projects(ctx: State<'_, AppContext>) -> Result<Vec<Project>, AppError> {
    ctx.projects.get_projects().map_err(AppError::from)
}

#[tauri::command]
pub fn seed_sample_project(ctx: State<'_, AppContext>) -> Result<Project, AppError> {
    let now = paths::now_ms();
    let id = ProjectId::from("p_sample_1".to_string());

    let project = Project {
        id: id.clone(),
        name: "demeteo-sample".to_string(),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 0,
        spend: 0.0,
        tokens: 0,
        created_at: now,
    };

    let _ = ctx.projects.add(project.clone());

    Ok(project)
}

#[tauri::command]
pub async fn update_project(
    ctx: State<'_, AppContext>,
    id: String,
    config: ProjectConfig,
) -> Result<(), AppError> {
    crate::application::projects::update(&ctx, id, config)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn delete_project(ctx: State<'_, AppContext>, id: String) -> Result<(), AppError> {
    crate::application::projects::delete_workspace(&ctx, id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn check_repos_dirty(
    ctx: State<'_, AppContext>,
    project_id: String,
    repo_paths: Vec<String>,
) -> Result<Vec<RepoDirtyStatus>, AppError> {
    crate::application::projects::check_repos_dirty(&ctx, project_id, repo_paths)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_repositories_for_project(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<Repository>, AppError> {
    ctx.projects
        .get_repositories_for(&ProjectId::from(project_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn get_workspace_health(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<RepoHealthStatus>, AppError> {
    crate::application::projects::health_check(&ctx, project_id)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_project_by_id(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Option<Project>, AppError> {
    ctx.projects
        .get_project(&ProjectId::from(project_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn resolve_repo_dir(
    ctx: State<'_, AppContext>,
    project_id: String,
    repo_path: String,
) -> Result<String, AppError> {
    let projects = ctx.projects.get_projects().map_err(AppError::from)?;
    let project_id_typed = ProjectId::from(project_id.clone());
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id_typed)
        .ok_or_else(|| AppError::not_found(format!("Project not found: {}", project_id)))?;
    crate::application::projects::resolve_target_dir(&ctx, &project, &project_id, &repo_path)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub fn project_memory_list(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<crate::domain::memory::ProjectMemoryEntry>, AppError> {
    ctx.memory
        .memory_list(&ProjectId::from(project_id), 100)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn project_memory_upsert(
    ctx: State<'_, AppContext>,
    id: Option<String>,
    project_id: String,
    key: String,
    value: String,
    source: String,
) -> Result<(), AppError> {
    let now = paths::now_ms();
    let source_enum = match source.as_str() {
        "agent" => crate::domain::memory::MemorySource::Agent,
        _ => crate::domain::memory::MemorySource::Human,
    };

    let resolved_id = if let Some(existing_id) = id {
        existing_id
    } else {
        let existing = ctx
            .memory
            .memory_list(&ProjectId::from(project_id.clone()), 100)
            .map_err(AppError::from)?;
        if let Some(found) = existing.iter().find(|e| e.key == key) {
            found.id.clone()
        } else {
            format!("pm-{}", paths::new_id())
        }
    };

    let entry = crate::domain::memory::ProjectMemoryEntry {
        id: resolved_id,
        project_id: ProjectId::from(project_id),
        key,
        value,
        source: source_enum,
        confidence: 1.0,
        memory_type: None,
        statement: None,
        embedding: None,
        embedding_model: None,
        last_used_at: None,
        use_count: 0,
        created_at: now,
        updated_at: now,
    };
    ctx.memory.memory_upsert(entry).map_err(AppError::from)
}

#[tauri::command]
pub fn project_memory_delete(ctx: State<'_, AppContext>, id: String) -> Result<(), AppError> {
    ctx.memory.memory_delete(&id).map_err(AppError::from)
}

/// List every workflow/step harness-model override configured for a project
/// (migrations V14/V15) — both workflow-level (`step_id = null`) and step-level
/// rows. The Project Settings "Workflow Overrides" tab calls this to hydrate
/// its rows; anything with no row inherits and won't appear here.
#[tauri::command]
pub fn get_workflow_overrides(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<crate::domain::models::ProjectWorkflowOverride>, AppError> {
    ctx.projects
        .list_workflow_overrides(&ProjectId::from(project_id))
        .map_err(AppError::from)
}

/// Upsert a single override. Scope is set by `step_id`: omit it (or pass an
/// empty string) for the workflow-level override; pass a step id to override
/// just that step. Passing `null` for both `agent_kind` and `model` clears the
/// override (the repo deletes the row), so it falls back to the inherited value.
#[tauri::command]
pub fn set_workflow_override(
    ctx: State<'_, AppContext>,
    project_id: String,
    workflow_id: String,
    step_id: Option<String>,
    agent_kind: Option<String>,
    model: Option<String>,
) -> Result<(), AppError> {
    // Normalise empty strings (the UI may send "") to None so they don't
    // masquerade as a real override / a real step target.
    let step_id = step_id.filter(|s| !s.trim().is_empty());
    let agent_kind = agent_kind.filter(|s| !s.trim().is_empty());
    let model = model.filter(|s| !s.trim().is_empty());
    ctx.projects
        .upsert_workflow_override(crate::domain::models::ProjectWorkflowOverride {
            project_id: ProjectId::from(project_id),
            workflow_id: crate::domain::ids::WorkflowId::from(workflow_id),
            step_id,
            agent_kind,
            model,
        })
        .map_err(AppError::from)
}
