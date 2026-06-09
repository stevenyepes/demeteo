use tauri::State;
use crate::state::DatabaseState;

#[tauri::command]
pub fn get_thread_events(state: State<'_, DatabaseState>, thread_id: String) -> Result<Vec<(serde_json::Value, i64)>, String> {
    state.db.get_thread_events(&thread_id)
}

#[tauri::command]
pub fn append_thread_event(
    state: State<'_, DatabaseState>,
    id: String,
    thread_id: String,
    event_json: String,
    seq: i64,
) -> Result<(), String> {
    state.db.append_thread_event(&id, &thread_id, &event_json, seq)
}