use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, RepoHealthStatus, Repository};
use crate::paths;
use crate::ports::execution::ExecutionPort;
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

/// Create a new project + its repositories. Shared by
/// `commands::project::create_project` (Tauri) and the headless runner's
/// `RunSpec` ingestion (M1.2) — same insert, one code path.
pub fn create(ctx: &AppContext, config: ProjectConfig) -> Result<Project, String> {
    let now = paths::now_ms();
    let id_str = format!("p{}", now);
    let id = ProjectId::from(id_str);

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

    ctx.projects.add(project.clone())?;

    for (i, repo_cfg) in config.repos.iter().enumerate() {
        let repo_id = RepositoryId::from(format!("{}_r{}", id.as_str(), i));
        let repo = Repository {
            id: repo_id,
            project_id: id.clone(),
            provider_id: ProviderId::from(repo_cfg.provider_id.clone()),
            repo_path: repo_cfg.repo_path.clone(),
        };
        ctx.projects.add_repository(repo)?;
    }

    Ok(project)
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
                eprintln!(
                    "[delete_project] could not resolve remote project root for {}: {}",
                    id, e
                );
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

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct LivenessResult {
    pub project_id: String,
    /// `"online"` or `"offline"`.
    pub liveness: String,
    /// ISO8601 UTC timestamp, e.g. `2026-07-11T00:00:00Z`.
    pub checked_at: String,
}

/// Probe whether the machine backing a project's workspace is currently
/// reachable. Resolves `compute_type`/`remote_host` exactly like
/// `health_check` above, then delegates the actual reachability check to
/// `ExecutionPort::test_connection` — for `compute_type == "local"` that is
/// a cheap no-op that always succeeds, so local workspaces resolve to
/// `"online"` instantly; for remote workspaces it opens/reuses an SSH
/// session. Any `Err` from the probe maps to `"offline"` — this function
/// itself only errors when the project id doesn't resolve.
pub async fn check_liveness(
    ctx: &AppContext,
    project_id: String,
) -> Result<LivenessResult, String> {
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
    let machine_str = machine_id.unwrap_or("local");

    Ok(liveness_result(ctx.exec.as_ref(), project_id, machine_str).await)
}

async fn liveness_result(
    exec: &dyn ExecutionPort,
    project_id: String,
    machine_id: &str,
) -> LivenessResult {
    let liveness = match exec.test_connection(machine_id).await {
        Ok(()) => "online",
        Err(_) => "offline",
    };
    LivenessResult {
        project_id,
        liveness: liveness.to_string(),
        checked_at: iso8601_now(),
    }
}

/// Current UTC time formatted as ISO8601 (`YYYY-MM-DDTHH:MM:SSZ`), computed
/// from `SystemTime` without pulling in a date/time crate dependency.
fn iso8601_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (year, month, day) = civil_from_unix_days(secs.div_euclid(86_400));
    let rem = secs.rem_euclid(86_400);
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, hour, minute, second
    )
}

/// Howard Hinnant's `civil_from_days`: days-since-Unix-epoch -> (year, month, day)
/// in the proleptic Gregorian calendar. http://howardhinnant.github.io/date_algorithms.html
fn civil_from_unix_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };
    (year, month, day)
}

#[cfg(test)]
#[path = "../../tests/application/projects.rs"]
mod tests;
