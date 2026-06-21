use crate::domain::ids::ThreadId;
use crate::domain::models::ThreadSession;
use crate::ports::db::ThreadPatch;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn get_thread_sessions(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<Vec<ThreadSession>, String> {
    ctx.threads
        .get_thread_sessions(&crate::domain::ids::MachineId::from(machine_id))
}

#[tauri::command]
pub fn add_thread_session(ctx: State<'_, AppContext>, thread: ThreadSession) -> Result<(), String> {
    ctx.threads.add_thread_session(thread.clone())?;

    if thread.mode == "worktree" {
        if let (Some(repo_path), Some(branch), Some(sandbox_path)) =
            (&thread.repo_path, &thread.branch, &thread.sandbox_path)
        {
            ctx.exec
                .setup_worktree(thread.machine_id.as_str(), repo_path, branch, sandbox_path)?;
        } else {
            return Err(
                "Missing worktree details (repo_path, branch, or sandbox_path)".to_string(),
            );
        }
    }
    Ok(())
}

#[tauri::command]
pub fn update_thread_status(
    ctx: State<'_, AppContext>,
    id: String,
    status: String,
) -> Result<(), String> {
    ctx.threads.update_thread(
        &ThreadId::from(id),
        &ThreadPatch {
            status: Some(status),
            ..Default::default()
        },
    )
}

#[tauri::command]
pub fn delete_thread_session(ctx: State<'_, AppContext>, id: String) -> Result<(), String> {
    ctx.threads.delete_thread_session(&ThreadId::from(id))
}
