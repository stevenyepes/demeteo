use tauri::State;
use crate::state::DatabaseState;
use crate::domain::models::Machine;

#[tauri::command]
pub fn get_machines(state: State<'_, DatabaseState>) -> Result<Vec<Machine>, String> {
    state.db.get_machines()
}

#[tauri::command]
pub fn add_machine(state: State<'_, DatabaseState>, machine: Machine) -> Result<(), String> {
    state.db.add_machine(machine)
}

#[tauri::command]
pub fn delete_machine(state: State<'_, DatabaseState>, id: String) -> Result<(), String> {
    state.db.delete_machine(&id)
}

#[tauri::command]
pub fn update_machine(state: State<'_, DatabaseState>, machine: Machine) -> Result<(), String> {
    state.db.update_machine(machine)
}
