use tauri::State;
use crate::state::AppContext;
use crate::domain::ids::ThreadId;
use crate::domain::models::Message;
use crate::ports::db::ThreadPatch;

#[tauri::command]
pub fn get_messages(ctx: State<'_, AppContext>, thread_id: String) -> Result<Vec<Message>, String> {
    ctx.threads.get_messages(&ThreadId::from(thread_id))
}

#[tauri::command]
pub fn append_message(ctx: State<'_, AppContext>, message: Message) -> Result<(), String> {
    ctx.threads.append_message(&message)?;
    ctx.threads.update_thread(&message.thread_id, &ThreadPatch { touch_timestamp: true, ..Default::default() })
}
