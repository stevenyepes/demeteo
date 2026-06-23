use crate::domain::ids::ProjectId;
use crate::domain::models::{Project, WorktreeStrategy};
use crate::paths;
use crate::state::AppContext;

fn get_repo_name(repo_path: &str) -> String {
    paths::repo_name_from_path(repo_path)
}

pub async fn bootstrap_project(
    ctx: &AppContext,
    project_id: String,
) -> Result<WorktreeStrategy, String> {
    // 1. Resolve project
    let projects = ctx.projects.get_projects()?;
    let project_id_typed = ProjectId::from(project_id.clone());
    let project = projects
        .into_iter()
        .find(|p| p.id == project_id_typed)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    // Update status to bootstrapping
    ctx.projects
        .update_status(&project_id_typed, "bootstrapping")?;

    match do_bootstrap_inner(ctx, &project_id, &project).await {
        Ok(strategy) => Ok(strategy),
        Err(err) => {
            let _ = ctx.projects.update_status(&project_id_typed, "error");
            Err(err)
        }
    }
}

async fn do_bootstrap_inner(
    ctx: &AppContext,
    project_id: &str,
    project: &Project,
) -> Result<WorktreeStrategy, String> {
    // 2. Resolve repos
    let project_id_typed = ProjectId::from(project_id.to_string());
    let repos = ctx.projects.get_repositories_for(&project_id_typed)?;
    if repos.is_empty() {
        return Err("No repositories configured for this project".to_string());
    }

    let mut main_repo_dir = String::new();

    // Determine machine_id (Some if remote, None if local)
    let machine_id = if project.compute_type.to_lowercase() == "local" {
        None
    } else {
        project.remote_host.as_ref().map(|m| m.as_str())
    };

    let is_local = project.compute_type.to_lowercase() == "local";
    let repos_parent_dir = if is_local {
        ctx.app_data_dir
            .join("projects")
            .join(project_id)
            .join("repos")
    } else {
        paths::project_root(
            &ctx.exec,
            &project.compute_type,
            project.remote_host.as_ref().map(|m| m.as_str()),
            project_id,
        )
        .await?
        .join(paths::REPOS_SUBDIR)
    };

    // Clean up any directories in the workspace repos/ folder that are no longer configured
    let machine_str = machine_id.unwrap_or("local");
    if project.compute_type.to_lowercase() == "local" {
        if repos_parent_dir.exists() {
            if let Ok(entries) = std::fs::read_dir(&repos_parent_dir) {
                for entry in entries.flatten() {
                    if let Ok(file_type) = entry.file_type() {
                        if file_type.is_dir() {
                            let dir_name = entry.file_name().to_string_lossy().to_string();
                            if !repos
                                .iter()
                                .any(|r| get_repo_name(&r.repo_path) == dir_name)
                            {
                                let _ = std::fs::remove_dir_all(entry.path());
                            }
                        }
                    }
                }
            }
        }
    } else {
        let allowed_names: Vec<String> =
            repos.iter().map(|r| get_repo_name(&r.repo_path)).collect();
        let allowed_names_str = allowed_names.join(" ");
        let repos_parent_str = repos_parent_dir.to_string_lossy().to_string();
        let cleanup_cmd = format!(
            "if [ -d \"{0}\" ]; then for d in \"{0}\"/*; do [ -d \"$d\" ] || continue; b=$(basename \"$d\"); match=0; for a in {1}; do if [ \"$b\" = \"$a\" ]; then match=1; break; fi; done; if [ $match -eq 0 ]; then rm -rf \"$d\"; fi; done; fi",
            repos_parent_str,
            allowed_names_str
        );
        let _ = ctx.exec.run_command(machine_str, &cleanup_cmd).await;
    }

    // 3. Loop and clone each repository
    for (i, repo) in repos.iter().enumerate() {
        let target_dir = if is_local {
            ctx.app_data_dir
                .join("projects")
                .join(project_id)
                .join("repos")
                .join(get_repo_name(&repo.repo_path))
                .to_string_lossy()
                .to_string()
        } else {
            paths::repo_target_dir_str(
                &ctx.exec,
                &project.compute_type,
                project.remote_host.as_ref().map(|m| m.as_str()),
                project_id,
                &repo.repo_path,
            )
            .await?
        };

        if i == 0 {
            main_repo_dir = target_dir.clone();
        }

        // Check if the directory already exists. Run a git command to test if clone is needed.
        let exists = ctx
            .exec
            .run_command(
                machine_str,
                &format!(
                    "git -C {} rev-parse --is-inside-work-tree",
                    paths::shell_escape_posix(&target_dir)
                ),
            )
            .await
            .is_ok();

        if !exists {
            ctx.worktree_ops
                .clone_repository(
                    machine_id,
                    repo.provider_id.as_str(),
                    &repo.repo_path,
                    &target_dir,
                )
                .await?;

            // Post-clone verification.
            let verified = ctx
                .exec
                .run_command(
                    machine_str,
                    &format!(
                        "git -C {} rev-parse --is-inside-work-tree",
                        paths::shell_escape_posix(&target_dir)
                    ),
                )
                .await
                .is_ok();
            if !verified {
                return Err(format!(
                    "Clone of '{}' reported success but '{}' is not a git working tree. \
                     Check the remote user's HOME, disk space, and repository URL.",
                    repo.repo_path, target_dir
                ));
            }
        }

        // Run machine-level setup commands after clone
        if let Some(cmds_json) = lookup_machine_setup_commands(ctx, machine_str) {
            for cmd in &cmds_json {
                let _ = ctx.exec.run_command(machine_str, cmd).await;
            }
        }
    }

    // 4. Run Strategy Detector on the main repository
    let strategy = ctx
        .worktree_ops
        .detect_worktree_strategy(machine_id, &main_repo_dir)
        .await?;

    Ok(strategy)
}

fn lookup_machine_setup_commands(ctx: &AppContext, machine_str: &str) -> Option<Vec<String>> {
    let machines = ctx.machines.get_machines().ok()?;
    let machine = machines.into_iter().find(|m| {
        m.id.as_ref() == machine_str
            || format!("{}@{}", m.username, m.host) == machine_str
            || m.host == machine_str
            || m.name == machine_str
    })?;
    let json = machine.setup_commands.as_ref()?;
    serde_json::from_str(json).ok()
}
