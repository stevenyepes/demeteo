use crate::domain::ids::MachineId;
use crate::domain::models::Machine;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn get_machines(ctx: State<'_, AppContext>) -> Result<Vec<Machine>, String> {
    ctx.machines.get_machines()
}

#[tauri::command]
pub fn add_machine(ctx: State<'_, AppContext>, machine: Machine) -> Result<(), String> {
    ctx.machines.add(machine)
}

#[tauri::command]
pub fn delete_machine(ctx: State<'_, AppContext>, id: String) -> Result<(), String> {
    ctx.machines.delete(&MachineId::from(id))
}

#[tauri::command]
pub fn update_machine(ctx: State<'_, AppContext>, machine: Machine) -> Result<(), String> {
    ctx.machines.update(machine)
}

#[tauri::command]
pub fn test_machine_connection(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<(), String> {
    ctx.exec.test_connection(&machine_id)
}
