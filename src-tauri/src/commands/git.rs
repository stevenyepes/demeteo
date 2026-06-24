use crate::error::AppError;
use crate::paths;
use crate::state::AppContext;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ChangedFile {
    pub path: String,
    /// "M" | "A" | "D" | "R" | "?"
    pub status: String,
}

/// List files changed between `base_ref` and `head_ref` in the given repo.
/// Runs: git diff --name-status <base_ref>...<head_ref>
#[tauri::command]
pub async fn git_changed_files(
    ctx: State<'_, AppContext>,
    machine_id: String,
    worktree_path: String,
    base_ref: String,
    head_ref: String,
) -> Result<Vec<ChangedFile>, AppError> {
    let cmd = format!(
        "git -C {} diff --name-status {}...{}",
        paths::shell_escape_posix(&worktree_path),
        paths::shell_escape_posix(&base_ref),
        paths::shell_escape_posix(&head_ref),
    );
    let output = ctx
        .exec
        .run_command(&machine_id, &cmd)
        .await
        .map_err(AppError::from)?;

    let files = output
        .lines()
        .filter_map(|line| {
            let mut parts = line.splitn(2, '\t');
            let raw_status = parts.next()?.trim();
            let path = parts.next()?.trim().to_string();
            // Rename entries look like "R100\told_path\tnew_path" after splitn(2)
            // so path may be "old\tnew" — take only the new path.
            let path = if raw_status.starts_with('R') {
                path.split('\t').last().unwrap_or(&path).to_string()
            } else {
                path
            };
            let status = raw_status.chars().next().unwrap_or('?').to_string();
            Some(ChangedFile { path, status })
        })
        .collect();

    Ok(files)
}

/// Return the content of `file_path` at `git_ref` in the worktree.
/// Returns an empty string when the file didn't exist at that ref (new file).
#[tauri::command]
pub async fn git_file_at_ref(
    ctx: State<'_, AppContext>,
    machine_id: String,
    worktree_path: String,
    git_ref: String,
    file_path: String,
) -> Result<String, AppError> {
    let cmd = format!(
        "git -C {} show {}:{} 2>/dev/null || true",
        paths::shell_escape_posix(&worktree_path),
        paths::shell_escape_posix(&git_ref),
        paths::shell_escape_posix(&file_path),
    );
    let output = ctx
        .exec
        .run_command(&machine_id, &cmd)
        .await
        .map_err(AppError::from)?;
    Ok(output)
}
