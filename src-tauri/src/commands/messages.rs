use crate::domain::ids::ThreadId;
use crate::domain::models::Message;
use crate::error::AppError;
use crate::ports::db::ThreadPatch;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn get_messages(
    ctx: State<'_, AppContext>,
    thread_id: String,
) -> Result<Vec<Message>, AppError> {
    // Read-model path (C3): the persisted agent stream is a display read, so it
    // flows through `RunView` (not `ThreadRepository` directly) — the seam C4
    // uses to source a runner-owned run's transcript from the laptop shadow.
    ctx.run_view
        .agent_stream(&ThreadId::from(thread_id))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn append_message(ctx: State<'_, AppContext>, message: Message) -> Result<(), AppError> {
    ctx.threads
        .append_message(&message)
        .map_err(AppError::from)?;
    ctx.threads
        .update_thread(
            &message.thread_id,
            &ThreadPatch {
                touch_timestamp: true,
                ..Default::default()
            },
        )
        .map_err(AppError::from)
}
