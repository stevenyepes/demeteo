use tauri::State;
use crate::state::DatabaseState;
use crate::domain::models::Message;

#[tauri::command]
pub fn get_messages(state: State<'_, DatabaseState>, thread_id: String) -> Result<Vec<Message>, String> {
    state.db.get_messages(&thread_id)
}

#[tauri::command]
pub fn append_message(state: State<'_, DatabaseState>, message: Message) -> Result<(), String> {
    state.db.append_message(&message)?;
    state.db.update_thread_timestamp(&message.thread_id)
}
