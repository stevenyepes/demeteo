use tauri::State;
use crate::state::DatabaseState;

#[tauri::command]
pub fn get_app_session(state: State<'_, DatabaseState>, key: String) -> Result<Option<String>, String> {
    state.db.get_app_session(&key)
}

#[tauri::command]
pub fn set_app_session(state: State<'_, DatabaseState>, key: String, value: String) -> Result<(), String> {
    state.db.set_app_session(&key, &value)
}

#[tauri::command]
pub fn delete_app_session(state: State<'_, DatabaseState>, key: String) -> Result<(), String> {
    state.db.delete_app_session(&key)
}