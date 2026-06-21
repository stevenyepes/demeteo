//! Tauri commands that wrap [`MrPublisher`].

use tauri::State;

use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, PublishOptions};
use crate::ports::mr_publisher::MrPublisher;
use crate::state::AppContext;

#[tauri::command]
pub async fn publish_mr(
    ctx: State<'_, AppContext>,
    project_id: String,
    feature_id: String,
    draft: Option<bool>,
    title: Option<String>,
    body: Option<String>,
) -> Result<MrInfo, String> {
    let options = PublishOptions {
        draft: draft.unwrap_or(false),
        title,
        body,
        target_branch: None,
    };
    ctx.mr_publisher
        .publish_mr(&project_id, &FeatureId::from(feature_id), options)
}

#[tauri::command]
pub async fn fetch_mr_state(
    ctx: State<'_, AppContext>,
    project_id: String,
    mr_url: String,
) -> Result<String, String> {
    ctx.mr_publisher.fetch_mr_state(&project_id, &mr_url)
}
