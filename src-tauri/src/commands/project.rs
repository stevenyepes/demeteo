use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, RepoHealthStatus, Repository};
use crate::paths;
use crate::state::AppContext;
use serde::{Deserialize, Serialize};
use tauri::{Manager, State};

/// Compute the absolute target dir for a (project, repo) pair.
///
/// Local projects live under Tauri's `app_local_data_dir()`; remote
/// projects live under the remote user's HOME (resolved via
/// `ExecutionPort::resolve_home`). This wrapper exists so the
/// `is_local` check lives in one place — every command that needs a
/// path must funnel through here, otherwise the bootstrap, health
/// check, and step executor can drift apart and the agent will end up
/// `cd`-ing into a directory the health check never probed.
fn resolve_target_dir(
    app: &tauri::AppHandle,
    exec: &std::sync::Arc<dyn crate::ports::execution::ExecutionPort>,
    project: &Project,
    project_id: &str,
    repo_path: &str,
) -> Result<String, String> {
    if project.compute_type.to_lowercase() == "local" {
        let local_data = app
            .path()
            .app_local_data_dir()
            .map_err(|e| format!("Failed to get local data dir: {}", e))?;
        let p = local_data
            .join("projects")
            .join(project_id)
            .join("repos")
            .join(paths::repo_name_from_path(repo_path));
        Ok(p.to_string_lossy().to_string())
    } else {
        paths::repo_target_dir_str(
            exec,
            &project.compute_type,
            project.remote_host.as_deref(),
            project_id,
            repo_path,
        )
    }
}

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
    ctx: State<'_, AppContext>,
    config: ProjectConfig,
) -> Result<ProjectCreateResponse, String> {
    let now = paths::now_ms();
    let id_str = format!("p{}", now);
    let id = ProjectId::from(id_str.clone());

    let project = Project {
        id: id.clone(),
        name: config.name.clone(),
        compute_type: config.compute_type.clone(),
        remote_host: config.remote_host.clone().map(MachineId::from),
        status: "bootstrapping".to_string(),
        nodes: if config.compute_type == "local" { 4 } else { 8 },
        spend: 0.0,
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
pub fn get_projects(ctx: State<'_, AppContext>) -> Result<Vec<Project>, String> {
    ctx.projects.get_projects()
}

#[tauri::command]
pub fn seed_sample_project(ctx: State<'_, AppContext>) -> Result<Project, String> {
    let now = paths::now_ms();
    let id = ProjectId::from("p_sample_1".to_string());

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
    let _ = ctx.projects.add(project.clone());

    Ok(project)
}

#[tauri::command]
pub async fn update_project(
    ctx: State<'_, AppContext>,
    id: String,
    config: ProjectConfig,
) -> Result<(), String> {
    // Fetch current project to preserve spend, created_at
    let existing_projects = ctx.projects.get_projects()?;
    let project_id = ProjectId::from(id.clone());
    let existing = existing_projects
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project {} not found", id))?;

    let updated_project = Project {
        id: project_id.clone(),
        name: config.name.clone(),
        compute_type: config.compute_type.clone(),
        remote_host: config.remote_host.clone().map(MachineId::from),
        status: "bootstrapping".to_string(),
        nodes: if config.compute_type == "local" { 4 } else { 8 },
        spend: existing.spend,
        created_at: existing.created_at,
    };

    ctx.projects.update(updated_project)?;

    // Re-create repositories entries for this project
    ctx.projects.delete_repositories_for(&project_id)?;
    for (i, repo_cfg) in config.repos.iter().enumerate() {
        let repo_id = RepositoryId::from(format!("{}_r{}", id, i));
        let repo = Repository {
            id: repo_id,
            project_id: project_id.clone(),
            provider_id: ProviderId::from(repo_cfg.provider_id.clone()),
            repo_path: repo_cfg.repo_path.clone(),
        };
        ctx.projects.add_repository(repo)?;
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_project(
    app: tauri::AppHandle,
    ctx: State<'_, AppContext>,
    id: String,
) -> Result<(), String> {
    use tauri::Manager;

    // Fetch project
    let projects = ctx.projects.get_projects()?;
    let project_id = ProjectId::from(id.clone());
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project {} not found", id))?;

    // Delete project from database
    ctx.projects.delete(&project_id)?;

    if project.compute_type.to_lowercase() == "local" {
        if let Ok(local_data) = app.path().app_local_data_dir() {
            let project_dir = local_data.join("projects").join(&id);
            if project_dir.exists() {
                let _ = std::fs::remove_dir_all(&project_dir);
            }
        }
    } else if let Some(machine_id) = &project.remote_host {
        // Use the shared helper so we delete exactly the directory the
        // bootstrap created — never a `~`-expanded guess.
        match paths::project_root(
            &ctx.exec,
            &project.compute_type,
            Some(machine_id.as_str()),
            &id,
        ) {
            Ok(remote_dir) => {
                let remote_dir_str = remote_dir.to_string_lossy().to_string();
                let _ = ctx.exec.run_command(
                    machine_id.as_str(),
                    &format!("rm -rf {}", paths::shell_escape_posix(&remote_dir_str)),
                );
            }
            Err(e) => {
                eprintln!(
                    "[delete_project] could not resolve remote project root for {}: {}",
                    id, e
                );
            }
        }
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
    ctx: State<'_, AppContext>,
    project_id: String,
    repo_paths: Vec<String>,
) -> Result<Vec<RepoDirtyStatus>, String> {
    use crate::adapters::worktree::git_ops::GitOpsHelper;

    let projects = ctx.projects.get_projects()?;
    let project_id_typed = ProjectId::from(project_id.clone());
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id_typed)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let machine_id = if project.compute_type.to_lowercase() == "local" {
        None
    } else {
        project.remote_host.as_ref().map(|m| m.as_str())
    };

    let git_ops = GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone());
    let mut results = Vec::new();

    for repo_path in repo_paths {
        let target_dir = resolve_target_dir(&app, &ctx.exec, &project, &project_id, &repo_path)?;

        let (has_uncommitted, has_unpushed) = git_ops
            .check_repo_dirty(machine_id, &target_dir)
            .unwrap_or((false, false));
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
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<Repository>, String> {
    ctx.projects
        .get_repositories_for(&ProjectId::from(project_id))
}

#[tauri::command]
pub async fn get_workspace_health(
    app: tauri::AppHandle,
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Vec<RepoHealthStatus>, String> {
    use crate::adapters::worktree::git_ops::GitOpsHelper;

    let projects = ctx.projects.get_projects()?;
    let project_id_typed = ProjectId::from(project_id.clone());
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id_typed)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let machine_id: Option<&str> = if project.compute_type.to_lowercase() == "local" {
        None
    } else {
        project.remote_host.as_ref().map(|m| m.as_str())
    };

    let repos = ctx.projects.get_repositories_for(&project_id_typed)?;
    let git_ops = GitOpsHelper::new(ctx.app_settings.clone(), ctx.exec.clone());
    let mut results = Vec::new();

    for repo in repos {
        let target_dir =
            resolve_target_dir(&app, &ctx.exec, &project, &project_id, &repo.repo_path)?;

        // Determine if the repo is actually cloned by checking for a git repo.
        // We log the exact path and the raw command output so a field
        // diagnosis can compare the path the backend *thinks* it is
        // probing against the path the user sees in their terminal.
        let machine_str = machine_id.unwrap_or("local");
        let probe_cmd = format!(
            "git -C {} rev-parse --is-inside-work-tree",
            paths::shell_escape_posix(&target_dir)
        );
        let probe_result = ctx.exec.run_command(machine_str, &probe_cmd);
        let is_cloned = probe_result.is_ok();
        eprintln!(
            "[get_workspace_health v2] project={} repo={} target_dir={} machine={} cmd={} ok={} stdout_or_err={:?}",
            project_id,
            repo.repo_path,
            target_dir,
            machine_str,
            probe_cmd,
            is_cloned,
            probe_result.as_ref().map(|s| s.as_str()).unwrap_or("<none>")
        );

        let head_branch = if is_cloned {
            git_ops.get_head_branch(machine_id, &target_dir)
        } else {
            None
        };

        let worktrees = if is_cloned {
            git_ops
                .list_worktrees(machine_id, &target_dir)
                .unwrap_or_default()
        } else {
            vec![]
        };

        let (has_uncommitted, has_unpushed) = if is_cloned {
            git_ops
                .check_repo_dirty(machine_id, &target_dir)
                .unwrap_or((false, false))
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

#[tauri::command]
pub fn get_project_by_id(
    ctx: State<'_, AppContext>,
    project_id: String,
) -> Result<Option<Project>, String> {
    ctx.projects.get_project(&ProjectId::from(project_id))
}
