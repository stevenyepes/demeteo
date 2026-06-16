use serde::{Deserialize, Serialize};
use tauri::State;
use crate::state::{DatabaseState, ExecutionState};
use crate::domain::models::{Project, Repository, RepoHealthStatus};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RepositoryConfig {
    pub repo_path: String,
    pub provider_id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProjectConfig {
    pub name: String,
    pub compute_type: String,
    pub remote_host: Option<String>,
    pub repos: Vec<RepositoryConfig>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ProjectCreateResponse {
    pub id: String,
    pub success: bool,
}

#[tauri::command]
pub async fn create_project(
    state: State<'_, DatabaseState>,
    config: ProjectConfig,
) -> Result<ProjectCreateResponse, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let id = format!("p{}", now);

    let project = Project {
        id: id.clone(),
        name: config.name.clone(),
        compute_type: config.compute_type.clone(),
        remote_host: config.remote_host.clone(),
        status: "bootstrapping".to_string(),
        nodes: if config.compute_type == "local" { 4 } else { 8 },
        spend: 0.0,
        created_at: now,
    };

    state.db.add_project(project)?;

    for (i, repo_cfg) in config.repos.iter().enumerate() {
        let repo_id = format!("{}_r{}", id, i);
        let repo = Repository {
            id: repo_id,
            project_id: id.clone(),
            provider_id: repo_cfg.provider_id.clone(),
            repo_path: repo_cfg.repo_path.clone(),
        };
        state.db.add_repository(repo)?;
    }

    Ok(ProjectCreateResponse {
        id,
        success: true,
    })
}

#[tauri::command]
pub fn get_projects(
    state: State<'_, DatabaseState>,
) -> Result<Vec<Project>, String> {
    state.db.get_projects()
}

#[tauri::command]
pub fn seed_sample_project(
    state: State<'_, DatabaseState>,
) -> Result<Project, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let id = "p_sample_1".to_string();

    let project = Project {
        id: id.clone(),
        name: "demeteo-sample".to_string(),
        compute_type: "local".to_string(),
        remote_host: None,
        status: "idle".to_string(),
        nodes: 4,
        spend: 0.0,
        created_at: now,
    };

    // Ignore error if it already exists
    let _ = state.db.add_project(project.clone());

    Ok(project)
}

#[tauri::command]
pub async fn update_project(
    state: State<'_, DatabaseState>,
    id: String,
    config: ProjectConfig,
) -> Result<(), String> {
    // Fetch current project to preserve spend, created_at
    let existing_projects = state.db.get_projects()?;
    let existing = existing_projects.into_iter().find(|p| p.id == id)
        .ok_or_else(|| format!("Project {} not found", id))?;

    let updated_project = Project {
        id: id.clone(),
        name: config.name.clone(),
        compute_type: config.compute_type.clone(),
        remote_host: config.remote_host.clone(),
        status: "bootstrapping".to_string(),
        nodes: if config.compute_type == "local" { 4 } else { 8 },
        spend: existing.spend,
        created_at: existing.created_at,
    };

    state.db.update_project(updated_project)?;

    // Re-create repositories entries for this project
    state.db.delete_repositories_for_project(&id)?;
    for (i, repo_cfg) in config.repos.iter().enumerate() {
        let repo_id = format!("{}_r{}", id, i);
        let repo = Repository {
            id: repo_id,
            project_id: id.clone(),
            provider_id: repo_cfg.provider_id.clone(),
            repo_path: repo_cfg.repo_path.clone(),
        };
        state.db.add_repository(repo)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_project(
    app: tauri::AppHandle,
    state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    id: String,
) -> Result<(), String> {
    use tauri::Manager;
    
    // Fetch project
    let projects = state.db.get_projects()?;
    let project = projects.into_iter().find(|p| p.id == id)
        .ok_or_else(|| format!("Project {} not found", id))?;

    // Delete project from database
    state.db.delete_project(&id)?;

    if project.compute_type.to_lowercase() == "local" {
        if let Ok(local_data) = app.path().app_local_data_dir() {
            let project_dir = local_data.join("projects").join(&id);
            if project_dir.exists() {
                let _ = std::fs::remove_dir_all(&project_dir);
            }
        }
    } else if let Some(machine_id) = &project.remote_host {
        let remote_dir = format!("~/.demeteo/projects/{}", id);
        let _ = exec_state.exec.run_command(machine_id, &format!("rm -rf \"{}\"", remote_dir));
    }

    Ok(())
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RepoDirtyStatus {
    pub repo_path: String,
    pub has_uncommitted: bool,
    pub has_unpushed: bool,
}

#[tauri::command]
pub async fn check_repos_dirty(
    app: tauri::AppHandle,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    project_id: String,
    repo_paths: Vec<String>,
) -> Result<Vec<RepoDirtyStatus>, String> {
    use tauri::Manager;
    use crate::adapters::worktree::git_ops::GitOpsHelper;

    let projects = db_state.db.get_projects()?;
    let project = projects.into_iter().find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let machine_id = if project.compute_type.to_lowercase() == "local" {
        None
    } else {
        project.remote_host.as_deref()
    };

    let git_ops = GitOpsHelper::new(db_state.db.clone(), exec_state.exec.clone());
    let mut results = Vec::new();

    for repo_path in repo_paths {
        let repo_name = repo_path.split('/').last().unwrap_or(&repo_path).to_string();
        let target_dir = if project.compute_type.to_lowercase() == "local" {
            let local_data = app.path().app_local_data_dir()
                .map_err(|e| format!("Failed to get local data dir: {}", e))?;
            local_data
                .join("projects")
                .join(&project_id)
                .join("repos")
                .join(&repo_name)
                .to_string_lossy()
                .to_string()
        } else {
            format!("~/.demeteo/projects/{}/repos/{}", project_id, repo_name)
        };

        let (has_uncommitted, has_unpushed) = git_ops.check_repo_dirty(machine_id, &target_dir).unwrap_or((false, false));
        results.push(RepoDirtyStatus {
            repo_path,
            has_uncommitted,
            has_unpushed,
        });
    }

    Ok(results)
}

#[tauri::command]
pub fn get_repositories_for_project(
    state: State<'_, DatabaseState>,
    project_id: String,
) -> Result<Vec<Repository>, String> {
    state.db.get_repositories_for_project(&project_id)
}

#[tauri::command]
pub async fn get_workspace_health(
    app: tauri::AppHandle,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    project_id: String,
) -> Result<Vec<RepoHealthStatus>, String> {
    use tauri::Manager;
    use crate::adapters::worktree::git_ops::GitOpsHelper;

    let projects = db_state.db.get_projects()?;
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let machine_id: Option<&str> = if project.compute_type.to_lowercase() == "local" {
        None
    } else {
        project.remote_host.as_deref()
    };

    let repos = db_state.db.get_repositories_for_project(&project_id)?;
    let git_ops = GitOpsHelper::new(db_state.db.clone(), exec_state.exec.clone());
    let mut results = Vec::new();

    for repo in repos {
        let repo_name = repo.repo_path.split('/').last().unwrap_or(&repo.repo_path).to_string();
        let target_dir = if project.compute_type.to_lowercase() == "local" {
            let local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| format!("Failed to get local data dir: {}", e))?;
            local_data
                .join("projects")
                .join(&project_id)
                .join("repos")
                .join(&repo_name)
                .to_string_lossy()
                .to_string()
        } else {
            format!("~/.demeteo/projects/{}/repos/{}", project_id, repo_name)
        };

        // Determine if the repo is actually cloned by checking for a git repo
        let is_cloned = exec_state
            .exec
            .run_command(
                machine_id.unwrap_or("local"),
                &format!("git -C \"{}\" rev-parse --is-inside-work-tree", target_dir),
            )
            .is_ok();

        let head_branch = if is_cloned {
            git_ops.get_head_branch(machine_id, &target_dir)
        } else {
            None
        };

        let worktrees = if is_cloned {
            git_ops.list_worktrees(machine_id, &target_dir).unwrap_or_default()
        } else {
            vec![]
        };

        let (has_uncommitted, has_unpushed) = if is_cloned {
            git_ops.check_repo_dirty(machine_id, &target_dir).unwrap_or((false, false))
        } else {
            (false, false)
        };

        results.push(RepoHealthStatus {
            repo_path: repo.repo_path,
            is_cloned,
            head_branch,
            worktrees,
            has_uncommitted,
            has_unpushed,
        });
    }

    Ok(results)
}
