use crate::domain::ids::MachineId;
use crate::domain::models::Machine;
use crate::error::AppError;
use crate::state::AppContext;
use tauri::State;

#[tauri::command]
pub fn get_machines(ctx: State<'_, AppContext>) -> Result<Vec<Machine>, AppError> {
    ctx.machines.get_machines().map_err(AppError::from)
}

#[tauri::command]
pub fn add_machine(ctx: State<'_, AppContext>, machine: Machine) -> Result<(), AppError> {
    ctx.machines.add(machine).map_err(AppError::from)
}

#[tauri::command]
pub fn delete_machine(ctx: State<'_, AppContext>, id: String) -> Result<(), AppError> {
    ctx.machines
        .delete(&MachineId::from(id))
        .map_err(AppError::from)
}

#[tauri::command]
pub fn update_machine(ctx: State<'_, AppContext>, machine: Machine) -> Result<(), AppError> {
    ctx.machines.update(machine).map_err(AppError::from)
}

#[tauri::command]
pub async fn test_machine_connection(
    ctx: State<'_, AppContext>,
    machine_id: String,
) -> Result<(), AppError> {
    ctx.exec
        .test_connection(&machine_id)
        .await
        .map_err(AppError::from)
}
