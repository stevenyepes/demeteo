use crate::domain::branch_listing::BranchOption;
use crate::domain::ids::{MachineId, ProjectId, ProviderId, RepositoryId};
use crate::domain::models::{Project, RepoHealthStatus, Repository, WorktreeInfo};
use crate::paths;
use crate::ports::worktree_ops::{TerminalWorktreeCreated, TerminalWorktreeRequest};
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

/// The places a terminal session may open inside one repository.
///
/// The main checkout is a *directory*, not a branch: a session opened there
/// inherits whatever HEAD it was last left on, which nothing in the app has
/// chosen and no listing of worktrees would reveal. So the two travel together
/// — the branch is the only part of that choice a picker cannot show on its
/// own, and a picker that cannot show it offers "main checkout" as if it named
/// a branch.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalLocations {
    /// The branch the main checkout is on, or `None` when it is detached or
    /// unreadable. Absent rather than guessed: naming the wrong branch here is
    /// worse than naming none, since the whole point is to be believed.
    pub main_branch: Option<String>,
    pub worktrees: Vec<WorktreeInfo>,
}

/// List the terminal locations of one repository of a project.
///
/// The project and repository IDs are the only authority accepted at this
/// boundary. Resolving the machine and checkout path here prevents a terminal
/// caller from directing Git operations to another project or host.
///
/// Only terminal-owned worktrees are returned; which those are is decided by
/// [`crate::domain::terminal_worktree`], not here. The listing must exclude the
/// checkouts a running pipeline step owns — those are torn down with
/// `worktree remove --force` under whoever opened a shell in one.
pub async fn list_terminal_locations(
    ctx: &AppContext,
    project_id: String,
    repository_id: String,
) -> Result<TerminalLocations, String> {
    let resolved = resolve_terminal_repository(ctx, &project_id, &repository_id).await?;
    let worktrees = ctx
        .worktree_ops
        .list_terminal_worktrees(
            resolved.machine_id.as_deref(),
            &resolved.repo_dir,
            &resolved.project_root,
        )
        .await?;
    // After the listing, so a repository the port refuses to read fails as
    // that refusal rather than as a missing branch name.
    let main_branch = ctx
        .worktree_ops
        .get_head_branch(resolved.machine_id.as_deref(), &resolved.repo_dir)
        .await
        .as_deref()
        .and_then(crate::domain::branch_listing::head_branch);

    Ok(TerminalLocations {
        main_branch,
        worktrees,
    })
}

/// The base branches a terminal worktree may be cut from, and which one this
/// project treats as its default.
///
/// The default travels with the list because it is the only part a picker
/// cannot derive: `refs/heads` says nothing about which branch this project
/// integrates into, and preselecting the wrong one is how a session quietly
/// starts from somewhere other than where work lands.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TerminalBranchOptions {
    pub default_branch: String,
    pub branches: Vec<BranchOption>,
}

/// List the base-branch candidates for one repository of a project.
pub async fn list_terminal_branches(
    ctx: &AppContext,
    project_id: String,
    repository_id: String,
) -> Result<TerminalBranchOptions, String> {
    let resolved = resolve_terminal_repository(ctx, &project_id, &repository_id).await?;
    let branches = ctx
        .worktree_ops
        .list_terminal_branches(resolved.machine_id.as_deref(), &resolved.repo_dir)
        .await?;

    Ok(TerminalBranchOptions {
        default_branch: resolved.default_branch,
        branches,
    })
}

/// Create a linked terminal worktree for one repository of a project.
///
/// Every field of `request` remains untrusted user input; the worktree adapter
/// validates them before deriving the final destination below the resolved
/// repository area.
pub async fn create_terminal_worktree(
    ctx: &AppContext,
    project_id: String,
    repository_id: String,
    request: TerminalWorktreeRequest,
) -> Result<TerminalWorktreeCreated, String> {
    let resolved = resolve_terminal_repository(ctx, &project_id, &repository_id).await?;
    ctx.worktree_ops
        .create_terminal_worktree(
            resolved.machine_id.as_deref(),
            &resolved.repo_dir,
            &resolved.project_root,
            &request,
        )
        .await
}

/// Remove one terminal worktree of a project's repository.
///
/// The path is resolved against this project's own repository before the port
/// sees it, exactly as the listing and creation paths are — a caller holding a
/// path from elsewhere must not be able to aim a removal through this project.
pub async fn remove_terminal_worktree(
    ctx: &AppContext,
    project_id: String,
    repository_id: String,
    worktree_path: String,
    force: bool,
) -> Result<(), String> {
    let resolved = resolve_terminal_repository(ctx, &project_id, &repository_id).await?;
    ctx.worktree_ops
        .remove_terminal_worktree(
            resolved.machine_id.as_deref(),
            &resolved.repo_dir,
            &resolved.project_root,
            &worktree_path,
            force,
        )
        .await
}

struct ResolvedTerminalRepository {
    machine_id: Option<String>,
    repo_dir: String,
    project_root: String,
    default_branch: String,
}

/// Resolve trusted terminal-worktree I/O inputs before calling the Git port.
/// Keeping this policy separate makes the ownership check testable without
/// constructing an execution driver or allowing command adapters to select a
/// host/path themselves.
async fn resolve_terminal_repository(
    ctx: &AppContext,
    project_id: &str,
    repository_id: &str,
) -> Result<ResolvedTerminalRepository, String> {
    let project_id_typed = ProjectId::from(project_id.to_string());
    let project = ctx
        .projects
        .get_project(&project_id_typed)?
        .ok_or_else(|| format!("Project not found: {project_id}"))?;
    let repository_id_typed = RepositoryId::from(repository_id.to_string());
    let repository = ctx
        .projects
        .get_repositories_for(&project_id_typed)?
        .into_iter()
        .find(|repository| repository.id == repository_id_typed)
        .ok_or_else(|| {
            format!("Repository {repository_id} does not belong to project {project_id}")
        })?;

    let machine_id = if project.compute_type.eq_ignore_ascii_case("local") {
        None
    } else {
        Some(
            project
                .remote_host
                .as_ref()
                .map(|machine| machine.as_str())
                .filter(|machine| !machine.trim().is_empty())
                .ok_or_else(|| format!("Remote project {project_id} has no configured machine"))?
                .to_string(),
        )
    };
    let repo_dir = resolve_target_dir(ctx, &project, project_id, &repository.repo_path).await?;
    // One call for both transports: the remote branch resolves `$HOME` over the
    // wire and ignores `workspace_dir`, so passing it is what keeps the local
    // and remote layouts structurally identical rather than coincidentally so.
    let project_root = paths::project_root(
        &ctx.exec,
        &project.compute_type,
        project.remote_host.as_deref(),
        project_id,
        Some(&ctx.workspace_dir),
    )
    .await?
    .to_string_lossy()
    .to_string();

    // The same settings a pipeline reads for its own base, falling back to the
    // shipped defaults exactly as `application::worktree` does — a terminal
    // session and a feature must not disagree about what this project
    // integrates into.
    let default_branch = ctx
        .projects
        .get_settings(&project_id_typed)?
        .unwrap_or_else(crate::adapters::step_executor::setup::fetch_default_settings)
        .worktree_strategy
        .default_branch;

    Ok(ResolvedTerminalRepository {
        machine_id,
        repo_dir,
        project_root,
        default_branch,
    })
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

/// The command settings the Strategy panel probes, as they stand **in the
/// form** — not as they stand in the database.
///
/// Sent by the caller rather than read back from storage because the whole
/// value of a configuration-time probe is answering for the command the user
/// just typed, before they have decided whether to keep it. A probe of the
/// saved record would tell them about the previous command.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct CommandProbeDraft {
    #[serde(default)]
    pub prepare_command: Option<String>,
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub harnesses: Option<std::collections::HashMap<String, String>>,
}

/// Probe a project's configured commands on **the project's own machine**
/// (HB6).
///
/// Which machine that is comes from the project, never from the caller: on a
/// remote-compute project the commands run on the runner, and an indicator that
/// silently asked the laptop instead would be confidently wrong exactly where
/// the answer matters most.
///
/// An indicator, not a gate — nothing here may block a save. The launch-time
/// gate (HB1/HB4) stays where it is.
pub async fn probe_commands(
    ctx: &AppContext,
    project_id: String,
    draft: CommandProbeDraft,
) -> Result<crate::adapters::step_executor::preflight::CommandProbeReport, String> {
    let projects = ctx.projects.get_projects()?;
    let project_id_typed = ProjectId::from(project_id.clone());
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id_typed)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    let machine = if project.compute_type.to_lowercase() == "local" {
        crate::domain::ids::LOCAL_MACHINE.to_string()
    } else {
        project
            .remote_host
            .as_ref()
            .map(|m| m.as_str())
            .filter(|m| !m.trim().is_empty())
            .ok_or_else(|| {
                "This project runs on a remote machine, but no machine is selected — the \
                 commands cannot be checked until one is."
                    .to_string()
            })?
            .to_string()
    };

    // Only the three command fields are read by the probe, so only those are
    // carried across. Spelling the rest out rather than loading the stored
    // strategy keeps this honest about what it looked at: a branch prefix or a
    // PR template has no bearing on whether `cargo` resolves.
    let strategy = crate::domain::models::WorktreeStrategy {
        default_branch: String::new(),
        branch_prefix: String::new(),
        test_command: draft.test_command,
        build_command: None,
        coverage_command: None,
        conventions_file: None,
        pr_template: None,
        harnesses: draft.harnesses,
        validation_gates: None,
        prepare_command: draft.prepare_command,
        extra_writable_paths: Vec::new(),
    };

    Ok(
        crate::adapters::step_executor::preflight::probe_project_commands(
            &*ctx.exec,
            &machine,
            &strategy,
            std::time::Duration::from_secs(
                crate::adapters::step_executor::preflight::PREFLIGHT_PROBE_TIMEOUT_S,
            ),
        )
        .await,
    )
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

        let machine_str = machine_id.unwrap_or(crate::domain::ids::LOCAL_MACHINE);
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
                .as_deref()
                .and_then(crate::domain::branch_listing::head_branch)
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

#[cfg(test)]
#[path = "../../tests/application/projects.rs"]
mod tests;
