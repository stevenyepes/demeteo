//! Tauri commands that wrap [`MrPublisher`].

use crate::error::AppError;
use tauri::State;

use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, PublishOptions};
use crate::domain::mr_summary::MrSummary;
use crate::state::AppContext;

#[tauri::command]
pub async fn publish_mr(
    ctx: State<'_, AppContext>,
    project_id: String,
    feature_id: String,
    draft: Option<bool>,
    title: Option<String>,
    body: Option<String>,
) -> Result<MrInfo, AppError> {
    let options = PublishOptions {
        draft: draft.unwrap_or(false),
        title,
        body,
        target_branch: None,
    };
    ctx.mr_publisher
        .publish_mr(&project_id, &FeatureId::from(feature_id), options)
        .await
        .map_err(AppError::from)
}

#[tauri::command]
pub async fn fetch_mr_state(
    ctx: State<'_, AppContext>,
    project_id: String,
    mr_url: String,
) -> Result<String, AppError> {
    ctx.mr_publisher
        .fetch_mr_state(&project_id, &mr_url)
        .await
        .map_err(AppError::from)
}

/// The open pull requests the Code Review view lists.
///
/// The `Err` is a JSON-serialized
/// [`MrListError`](crate::domain::mr_list_error::MrListError), not the
/// `AppError` sentence every command beside it produces. That module records
/// why: the four failures differ in what the user should *do*, and the facts
/// separating them — which host, which status, how long the limit has left —
/// do not survive a `.to_string()`. `src/lib/pullRequests.ts` decodes it, and
/// tests on both sides quote the same literals so a rename cannot land on one
/// side alone.
#[tauri::command]
pub async fn list_open_pull_requests(
    ctx: State<'_, AppContext>,
    project_id: String,
    repository_id: Option<String>,
) -> Result<Vec<MrSummary>, String> {
    ctx.mr_publisher
        .list_open_mrs(&project_id, repository_id.as_deref())
        .await
        .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))
}

/// Post `body` on the pull request at `pull_request_url`, answering with the
/// created comment's URL.
///
/// Every other command here can be re-issued after a failure; this one reaches
/// a service Demeteo does not own and leaves something a stranger will read, so
/// the confirmation that guards it is the frontend's and there is no second
/// gate behind this call. See
/// [`MrPublisher::post_mr_comment`](crate::ports::mr_publisher::MrPublisher::post_mr_comment).
#[tauri::command]
pub async fn post_pull_request_comment(
    ctx: State<'_, AppContext>,
    project_id: String,
    pull_request_url: String,
    body: String,
) -> Result<String, AppError> {
    ctx.mr_publisher
        .post_mr_comment(&project_id, &pull_request_url, &body)
        .await
        .map_err(AppError::from)
}
