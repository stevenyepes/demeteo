use tauri::State;
use crate::state::DatabaseState;
use crate::domain::models::Feature;

#[tauri::command]
pub fn fetch_active_features(
    state: State<'_, DatabaseState>,
    project_id: String,
) -> Result<Vec<Feature>, String> {
    state.db.get_active_features(&project_id)
}

#[tauri::command]
pub fn start_feature(
    state: State<'_, DatabaseState>,
    project_id: String,
    title: String,
) -> Result<Feature, String> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
    let id = format!("f{}", now);

    let feature = Feature {
        id: id.clone(),
        project_id,
        title,
        status: "running".to_string(),
        total_cost: 0.0,
        duration: "0s".to_string(),
        created_at: now,
    };

    state.db.add_feature(feature.clone())?;

    Ok(feature)
}
