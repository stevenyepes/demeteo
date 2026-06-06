use crate::ExecutionState;
use serde::Serialize;
use tauri::State;

#[derive(Serialize, Clone)]
pub struct SftpEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64,
}

#[tauri::command]
pub fn sftp_list_dir(
    state: State<'_, ExecutionState>,
    machine_id: String,
    path: String,
) -> Result<Vec<SftpEntry>, String> {
    state.exec.list_dir(&machine_id, &path)
}

#[tauri::command]
pub fn sftp_read_file(
    state: State<'_, ExecutionState>,
    machine_id: String,
    path: String,
) -> Result<String, String> {
    state.exec.read_file(&machine_id, &path)
}

#[tauri::command]
pub fn sftp_write_file(
    state: State<'_, ExecutionState>,
    machine_id: String,
    path: String,
    content: String,
) -> Result<(), String> {
    state.exec.write_file(&machine_id, &path, &content)
}

#[tauri::command]
pub fn sftp_get_metadata(
    state: State<'_, ExecutionState>,
    machine_id: String,
    path: String,
) -> Result<SftpEntry, String> {
    state.exec.get_metadata(&machine_id, &path)
}
