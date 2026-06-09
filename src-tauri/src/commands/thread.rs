use tauri::State;
use crate::state::{DatabaseState, ExecutionState};
use crate::domain::models::ThreadSession;

#[tauri::command]
pub fn get_thread_sessions(state: State<'_, DatabaseState>, machine_id: String) -> Result<Vec<ThreadSession>, String> {
    state.db.get_thread_sessions(&machine_id)
}

#[tauri::command]
pub fn add_thread_session(
    db_state: State<'_, DatabaseState>,
    exec_state: State<'_, ExecutionState>,
    thread: ThreadSession,
) -> Result<(), String> {
    db_state.db.add_thread_session(thread.clone())?;

    if thread.mode == "worktree" {
        if let (Some(repo_path), Some(branch), Some(sandbox_path)) = (&thread.repo_path, &thread.branch, &thread.sandbox_path) {
            exec_state.exec.setup_worktree(&thread.machine_id, repo_path, branch, sandbox_path)?;
        } else {
            return Err("Missing worktree details (repo_path, branch, or sandbox_path)".to_string());
        }
    }
    Ok(())
}

#[tauri::command]
pub fn update_thread_status(
    state: State<'_, DatabaseState>,
    id: String,
    status: String,
) -> Result<(), String> {
    state.db.update_thread_status(&id, &status)
}

#[tauri::command]
pub fn delete_thread_session(state: State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_thread_session(&id)
}
