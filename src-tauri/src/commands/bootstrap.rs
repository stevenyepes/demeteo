use tauri::{State, Manager};
use crate::state::{DatabaseState, ExecutionState};
use crate::domain::models::{ProjectSettings, WorktreeStrategy};
use crate::adapters::worktree::git_ops::GitOpsHelper;

fn get_repo_name(repo_path: &str) -> String {
    repo_path.split('/').last().unwrap_or(repo_path).to_string()
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

    // Clean up any directories in the workspace repos/ folder that are no longer configured
    let machine_str = machine_id.unwrap_or("local");
    if project.compute_type.to_lowercase() == "local" {
        if let Ok(local_data) = app.path().app_local_data_dir() {
            let repos_parent_dir = local_data.join("projects").join(project_id).join("repos");
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
        }
    } else {
        let repos_parent_dir = format!("~/.demeteo/projects/{}/repos", project_id);
        let allowed_names: Vec<String> = repos.iter().map(|r| get_repo_name(&r.repo_path)).collect();
        let allowed_names_str = allowed_names.join(" ");
        let cleanup_cmd = format!(
            "if [ -d \"{0}\" ]; then for d in \"{0}\"/*; do [ -d \"$d\" ] || continue; b=$(basename \"$d\"); match=0; for a in {1}; do if [ \"$b\" = \"$a\" ]; then match=1; break; fi; done; if [ $match -eq 0 ]; then rm -rf \"$d\"; fi; done; fi",
            repos_parent_dir,
            allowed_names_str
        );
        let _ = exec_state.exec.run_command(machine_str, &cleanup_cmd);
    }

    // 3. Loop and clone each repository
    for (i, repo) in repos.iter().enumerate() {
        let repo_name = get_repo_name(&repo.repo_path);
        
        let target_dir = if project.compute_type.to_lowercase() == "local" {
            let local_data = app.path().app_local_data_dir()
                .map_err(|e| format!("Failed to get local data dir: {}", e))?;
            local_data
                .join("projects")
                .join(project_id)
                .join("repos")
                .join(&repo_name)
                .to_string_lossy()
                .to_string()
        } else {
            format!("~/.demeteo/projects/{}/repos/{}", project_id, repo_name)
        };

        if i == 0 {
            main_repo_dir = target_dir.clone();
        }

        // Check if the directory already exists. Run a git command to test if clone is needed.
        let machine_str = machine_id.unwrap_or("local");
        let exists = exec_state.exec.run_command(machine_str, &format!("git -C \"{}\" rev-parse --is-inside-work-tree", target_dir)).is_ok();

        if !exists {
            git_ops.clone_repository(machine_id, &repo.provider_id, &repo.repo_path, &target_dir)?;
        }
    }

    // 4. Run Strategy Detector on the main repository
    let strategy = git_ops.detect_worktree_strategy(machine_id, &main_repo_dir)?;

    Ok(strategy)
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
