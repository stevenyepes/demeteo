use crate::domain::ids::{FeatureId, MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, RepoHealthStatus, Repository};
use crate::paths;
use crate::state::AppContext;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RepositoryConfig {
    pub repo_path: String,
    pub provider_id: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub compute_type: String,
    pub remote_host: Option<String>,
    pub repos: Vec<RepositoryConfig>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RepoDirtyStatus {
    pub repo_path: String,
    pub has_uncommitted: bool,
    pub has_unpushed: bool,
}

/// Compute the absolute target dir for a (project, repo) pair.
pub async fn resolve_target_dir(
    ctx: &AppContext,
    project: &Project,
    project_id: &str,
    repo_path: &str,
) -> Result<String, String> {
    if project.compute_type.to_lowercase() == "local" {
        Ok(
            paths::repo_target_dir_local(&ctx.workspace_dir, project_id, repo_path)
                .to_string_lossy()
                .to_string(),
        )
    } else {
        paths::repo_target_dir_str(
            &ctx.exec,
            &project.compute_type,
            project.remote_host.as_deref(),
            project_id,
            repo_path,
            None,
        )
        .await
    }
}

pub async fn update(ctx: &AppContext, id: String, config: ProjectConfig) -> Result<(), String> {
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
        nodes: existing.nodes,
        spend: existing.spend,
        tokens: existing.tokens,
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

pub async fn delete_workspace(ctx: &AppContext, id: String) -> Result<(), String> {
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
        let project_dir = paths::project_root_local(&ctx.workspace_dir, &id);
        if project_dir.exists() {
            let _ = std::fs::remove_dir_all(&project_dir);
        }
    } else if let Some(machine_id) = &project.remote_host {
        // Use the shared helper so we delete exactly the directory the
        // bootstrap created — never a `~`-expanded guess.
        match paths::project_root(
            &ctx.exec,
            &project.compute_type,
            Some(machine_id.as_str()),
            &id,
            None,
        )
        .await
        {
            Ok(remote_dir) => {
                let remote_dir_str = remote_dir.to_string_lossy().to_string();
                let _ = ctx
                    .exec
                    .run_command(
                        machine_id.as_str(),
                        &format!("rm -rf {}", paths::shell_escape_posix(&remote_dir_str)),
                    )
                    .await;
            }
            Err(e) => {
                tracing::warn!(project_id = %id, error = %e, "could not resolve remote project root");
            }
        }
    }

    Ok(())
}

pub async fn check_repos_dirty(
    ctx: &AppContext,
    project_id: String,
    repo_paths: Vec<String>,
) -> Result<Vec<RepoDirtyStatus>, String> {
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

    let mut results = Vec::new();

    for repo_path in repo_paths {
        let target_dir = resolve_target_dir(ctx, &project, &project_id, &repo_path).await?;

        let (has_uncommitted, has_unpushed) = ctx
            .worktree_ops
            .check_repo_dirty(machine_id, &target_dir)
            .await
            .unwrap_or((false, false));
        results.push(RepoDirtyStatus {
            repo_path,
            has_uncommitted,
            has_unpushed,
        });
    }

    Ok(results)
}

pub async fn health_check(
    ctx: &AppContext,
    project_id: String,
) -> Result<Vec<RepoHealthStatus>, String> {
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
    let mut results = Vec::new();

    for repo in repos {
        let target_dir = resolve_target_dir(ctx, &project, &project_id, &repo.repo_path).await?;

        let machine_str = machine_id.unwrap_or("local");
        let probe_cmd = format!(
            "git -C {} rev-parse --is-inside-work-tree",
            paths::shell_escape_posix(&target_dir)
        );
        let probe_result = ctx.exec.run_command(machine_str, &probe_cmd).await;
        let is_cloned = probe_result.is_ok();
        tracing::debug!(
            project_id,
            repo = %repo.repo_path,
            target_dir,
            machine = machine_str,
            ok = is_cloned,
            "get_workspace_health probe"
        );

        let head_branch = if is_cloned {
            ctx.worktree_ops
                .get_head_branch(machine_id, &target_dir)
                .await
        } else {
            None
        };

        let worktrees = if is_cloned {
            ctx.worktree_ops
                .list_worktrees(machine_id, &target_dir)
                .await
                .unwrap_or_default()
        } else {
            vec![]
        };

        let (has_uncommitted, has_unpushed) = if is_cloned {
            ctx.worktree_ops
                .check_repo_dirty(machine_id, &target_dir)
                .await
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

#[derive(Serialize)]
pub struct WorktreeInfo {
    pub machine_id: String,
    pub worktree_path: String,
    pub branch: String,
    pub default_branch: String,
}

pub async fn worktree_info(ctx: &AppContext, feature_id: String) -> Result<WorktreeInfo, String> {
    let fid = FeatureId::from(feature_id);
    let feature = ctx
        .features
        .get(&fid)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "Feature not found".to_string())?;

    let project_id = feature.project_id;
    let all = ctx.projects.get_projects().map_err(|e| e.to_string())?;
    let project = all
        .into_iter()
        .find(|p| p.id == project_id)
        .ok_or_else(|| "Project not found".to_string())?;

    let repos = ctx
        .projects
        .get_repositories_for(&project_id)
        .map_err(|e| e.to_string())?;
    let repo = repos
        .first()
        .ok_or_else(|| "No repository configured for this project".to_string())?;

    let settings = ctx
        .projects
        .get_settings(&project_id)
        .map_err(|e| e.to_string())?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings);

    let machine_id = if project.compute_type.eq_ignore_ascii_case("local") {
        "local".to_string()
    } else {
        project
            .remote_host
            .as_ref()
            .map(|m| m.as_str().to_string())
            .unwrap_or_else(|| "local".to_string())
    };

    let worktree_path = if project.compute_type.eq_ignore_ascii_case("local") {
        paths::repo_target_dir_local(&ctx.workspace_dir, &project_id.0, &repo.repo_path)
            .to_string_lossy()
            .to_string()
    } else {
        paths::repo_target_dir_str(
            &ctx.exec,
            &project.compute_type,
            project.remote_host.as_ref().map(|m| m.as_str()),
            &project_id.0,
            &repo.repo_path,
            None,
        )
        .await
        .map_err(|e| e.to_string())?
    };

    let branch = format!("{}{}", settings.worktree_strategy.branch_prefix, fid.0);

    Ok(WorktreeInfo {
        machine_id,
        worktree_path,
        branch,
        default_branch: settings.worktree_strategy.default_branch.clone(),
    })
}
