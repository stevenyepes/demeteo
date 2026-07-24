use super::rpc::remote_rpc;
use crate::error::AppError;
use crate::state::AppContext;

pub async fn get_status(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "get_status",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)
}

pub async fn stream_events(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    from_offset: i64,
) -> Result<serde_json::Value, AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "stream_events",
        serde_json::json!({ "run_id": run_id, "from_offset": from_offset }),
    )
    .await
    .map_err(AppError::from)
}

pub async fn get_feature(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "get_feature",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)
}

pub async fn list_steps(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "list_steps",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)
}

pub async fn read_artifact(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    path: String,
) -> Result<serde_json::Value, AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "read_artifact",
        serde_json::json!({ "run_id": run_id, "path": path }),
    )
    .await
    .map_err(AppError::from)
}

pub async fn list_messages(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    thread_id: String,
) -> Result<serde_json::Value, AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "list_messages",
        serde_json::json!({ "run_id": run_id, "thread_id": thread_id }),
    )
    .await
    .map_err(AppError::from)
}

pub async fn get_worktree(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<serde_json::Value, AppError> {
    let mut info = remote_rpc(
        ctx,
        &machine_id,
        "get_worktree",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)?;
    if let Some(object) = info.as_object_mut() {
        object.insert("machine_id".to_string(), serde_json::json!(machine_id));
    }
    Ok(info)
}

pub async fn decide_gate(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    gate_id: String,
    decision: String,
    feedback: Option<String>,
) -> Result<(), AppError> {
    remote_rpc(
        ctx,
        &machine_id,
        "decide_gate",
        serde_json::json!({
            "run_id": run_id,
            "gate_id": gate_id,
            "decision": decision,
            "feedback": feedback,
        }),
    )
    .await
    .map(|_| ())
    .map_err(AppError::from)
}
