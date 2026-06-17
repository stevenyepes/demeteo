use tauri::{State, Manager};
use crate::state::{DatabaseState, ExecutionState};
use crate::domain::models::{ProjectSettings, WorktreeStrategy};
use crate::adapters::worktree::git_ops::GitOpsHelper;
use crate::paths;

fn get_repo_name(repo_path: &str) -> String {
    paths::repo_name_from_path(repo_path)
}

#[tauri::command]
pub async fn bootstrap_project(
    app: tauri::AppHandle,
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    project_id: String,
) -> Result<WorktreeStrategy, String> {
    // 1. Resolve project
    let projects = db_state.db.get_projects()?;
    let project = projects.into_iter().find(|p| p.id == project_id)
        .ok_or_else(|| format!("Project not found: {}", project_id))?;

    // Update status to bootstrapping
    db_state.db.update_project_status(&project_id, "bootstrapping")?;

    match do_bootstrap_inner(&app, &db_state, &exec_state, &project_id, &project).await {
        Ok(strategy) => Ok(strategy),
        Err(err) => {
            let _ = db_state.db.update_project_status(&project_id, "error");
            Err(err)
        }
    }
}

async fn do_bootstrap_inner(
    app: &tauri::AppHandle,
    db_state: &State<'_, DatabaseState>,
    exec_state: &State<'_, ExecutionState>,
    project_id: &str,
    project: &crate::domain::models::Project,
) -> Result<WorktreeStrategy, String> {
    // 2. Resolve repos
    let repos = db_state.db.get_repositories_for_project(project_id)?;
    if repos.is_empty() {
        return Err("No repositories configured for this project".to_string());
    }

    let git_ops = GitOpsHelper::new(db_state.db.clone(), exec_state.exec.clone());
    let mut main_repo_dir = String::new();

    // Determine machine_id (Some if remote, None if local)
    let machine_id = if project.compute_type.to_lowercase() == "local" {
        None
    } else {
        project.remote_host.as_deref()
    };

    // Resolve the absolute repos parent dir *once* for this project. We
    // deliberately avoid `~/.demeteo/...` here because `~` expansion in
    // the remote shell is fragile (HOME unset, user renamed, custom
    // passwd entry). See `paths::project_root` for the rationale.
    //
    // Local projects live under Tauri's `app_local_data_dir()` (e.g.
    // `~/.local/share/demeteo/projects/<id>`); remote projects live
    // under the remote user's `$HOME/.demeteo/projects/<id>`. The two
    // bases are intentionally different so a single `rm -rf` on
    // either side can't wipe the other.
    let is_local = project.compute_type.to_lowercase() == "local";
    let repos_parent_dir = if is_local {
        let local_data = app
            .path()
            .app_local_data_dir()
            .map_err(|e| format!("Failed to get local data dir: {}", e))?;
        local_data.join("projects").join(project_id).join("repos")
    } else {
        paths::project_root(
            &exec_state.exec,
            &project.compute_type,
            project.remote_host.as_deref(),
            project_id,
        )?
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
                            if !repos.iter().any(|r| get_repo_name(&r.repo_path) == dir_name) {
                                let _ = std::fs::remove_dir_all(entry.path());
                            }
                        }
                    }
                }
            }
        }
    } else {
        let allowed_names: Vec<String> = repos.iter().map(|r| get_repo_name(&r.repo_path)).collect();
        let allowed_names_str = allowed_names.join(" ");
        let repos_parent_str = repos_parent_dir.to_string_lossy().to_string();
        let cleanup_cmd = format!(
            "if [ -d \"{0}\" ]; then for d in \"{0}\"/*; do [ -d \"$d\" ] || continue; b=$(basename \"$d\"); match=0; for a in {1}; do if [ \"$b\" = \"$a\" ]; then match=1; break; fi; done; if [ $match -eq 0 ]; then rm -rf \"$d\"; fi; done; fi",
            repos_parent_str,
            allowed_names_str
        );
        let _ = exec_state.exec.run_command(machine_str, &cleanup_cmd);
    }

    // 3. Loop and clone each repository
    for (i, repo) in repos.iter().enumerate() {
        let target_dir = if is_local {
            let local_data = app
                .path()
                .app_local_data_dir()
                .map_err(|e| format!("Failed to get local data dir: {}", e))?;
            local_data
                .join("projects")
                .join(project_id)
                .join("repos")
                .join(get_repo_name(&repo.repo_path))
                .to_string_lossy()
                .to_string()
        } else {
            paths::repo_target_dir_str(
                &exec_state.exec,
                &project.compute_type,
                project.remote_host.as_deref(),
                project_id,
                &repo.repo_path,
            )?
        };

        if i == 0 {
            main_repo_dir = target_dir.clone();
        }

        // Check if the directory already exists. Run a git command to test if clone is needed.
        let exists = exec_state
            .exec
            .run_command(
                machine_str,
                &format!("git -C {} rev-parse --is-inside-work-tree", shell_escape(&target_dir)),
            )
            .is_ok();

        if !exists {
            git_ops.clone_repository(
                machine_id,
                &repo.provider_id,
                &repo.repo_path,
                &target_dir,
            )?;

            // Post-clone verification. `clone_repository` can return Ok
            // for partial failures (e.g. a non-empty target dir that
            // contains an unrelated repo), and we want to fail loudly
            // now rather than surface a confusing `agent closed stdout`
            // from the step executor ten seconds later.
            let verified = exec_state
                .exec
                .run_command(
                    machine_str,
                    &format!("git -C {} rev-parse --is-inside-work-tree", shell_escape(&target_dir)),
                )
                .is_ok();
            if !verified {
                return Err(format!(
                    "Clone of '{}' reported success but '{}' is not a git working tree. \
                     Check the remote user's HOME, disk space, and repository URL.",
                    repo.repo_path, target_dir
                ));
            }
        }
    }

    // 4. Run Strategy Detector on the main repository
    let strategy = git_ops.detect_worktree_strategy(machine_id, &main_repo_dir)?;

    Ok(strategy)
}

/// Single-quote-escape a path for use in a POSIX shell command. We avoid
/// double-quoting so `~` is not interpreted by the shell, and we use
/// single-quote-escape so paths with spaces / quotes are handled
/// safely. For our use case the path is always ASCII with only `~`,
/// `.`, `/`, `_`, and alphanumerics, so the fast path returns it
/// verbatim; the quoted fallback is defensive.
fn shell_escape(s: &str) -> String {
    if s.is_empty() {
        return "''".into();
    }
    if s.chars().all(|c| c.is_ascii_alphanumeric()
        || matches!(c, '_' | '-' | '.' | '/' | '=' | ':' | ',' | '@' | '~'))
    {
        return s.to_string();
    }
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}


#[tauri::command]
pub fn get_proposed_strategy(
    db_state: State<'_, DatabaseState>,
    project_id: String,
) -> Result<Option<ProjectSettings>, String> {
    db_state.db.get_project_settings(&project_id)
}

#[tauri::command]
pub fn save_project_settings(
    db_state: State<'_, DatabaseState>,
    project_id: String,
    settings: ProjectSettings,
) -> Result<(), String> {
    // Save to DB
    db_state.db.save_project_settings(settings)?;

    // Set project status to idle (workspace build complete)
    db_state.db.update_project_status(&project_id, "idle")?;

    Ok(())
}
