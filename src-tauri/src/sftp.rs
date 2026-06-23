use crate::state::AppContext;
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
pub async fn sftp_list_dir(
    ctx: State<'_, AppContext>,
    machine_id: String,
    path: String,
) -> Result<Vec<SftpEntry>, String> {
    ctx.exec.list_dir(&machine_id, &path).await
}

#[tauri::command]
pub async fn sftp_read_file(
    ctx: State<'_, AppContext>,
    machine_id: String,
    path: String,
) -> Result<String, String> {
    ctx.exec.read_file(&machine_id, &path).await
}

#[tauri::command]
pub async fn sftp_write_file(
    ctx: State<'_, AppContext>,
    machine_id: String,
    path: String,
    content: String,
) -> Result<(), String> {
    ctx.exec.write_file(&machine_id, &path, &content).await
}

#[tauri::command]
pub async fn sftp_get_metadata(
    ctx: State<'_, AppContext>,
    machine_id: String,
    path: String,
) -> Result<SftpEntry, String> {
    ctx.exec.get_metadata(&machine_id, &path).await
}
