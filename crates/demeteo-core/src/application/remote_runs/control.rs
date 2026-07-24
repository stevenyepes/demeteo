use super::credentials::inject_pat_for_run;
use super::reconcile::{reconcile_one_run, NOTIFY_ON};
use super::rpc::{json_str, remote_rpc};
use crate::domain::models::EffortLevel;
use crate::error::AppError;
use crate::ports::remote_run_mirror::RemoteRunMirror;
use crate::state::AppContext;

pub async fn cancel_remote_run(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<(), AppError> {
    let result = remote_rpc(
        ctx,
        &machine_id,
        "cancel_run",
        serde_json::json!({ "run_id": run_id }),
    )
    .await
    .map_err(AppError::from)?;
    let status = json_str(&result, "status").unwrap_or_else(|| "cancelled".to_string());
    ctx.remote_run_mirror
        .update_status(
            &machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            crate::paths::now_ms(),
        )
        .map_err(AppError::from)?;
    Ok(())
}

pub async fn retry_remote_step(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
    step_execution_id: String,
    model: Option<String>,
    agent_kind: Option<String>,
    effort: Option<EffortLevel>,
) -> Result<(), AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Err(AppError::not_found(format!(
            "No detached run {run_id} on machine {machine_id}"
        )));
    };
    inject_pat_for_run(ctx, &machine_id, &run_id, &row).await?;
    let result = remote_rpc(
        ctx,
        &machine_id,
        "retry_step",
        serde_json::json!({
            "run_id": run_id,
            "step_execution_id": step_execution_id,
            "model": model,
            "agent_kind": agent_kind,
            "effort": effort,
        }),
    )
    .await
    .map_err(AppError::from)?;
    let status = json_str(&result, "status").unwrap_or_else(|| "running".to_string());
    ctx.remote_run_mirror
        .update_status(
            &machine_id,
            &run_id,
            &status,
            None,
            None,
            None,
            None,
            0,
            crate::paths::now_ms(),
        )
        .map_err(AppError::from)?;
    reconcile_one_run(ctx, &row).await;
    Ok(())
}

pub async fn reinject_credentials(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    inject_pat_for_run(ctx, &machine_id, &run_id, &row).await?;
    reconcile_one_run(ctx, &row).await;
    ctx.remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)
}

pub async fn refresh_remote_run(
    ctx: &AppContext,
    machine_id: String,
    run_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    let Some(row) = ctx
        .remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)?
    else {
        return Ok(None);
    };
    reconcile_one_run(ctx, &row).await;
    ctx.remote_run_mirror
        .get(&machine_id, &run_id)
        .map_err(AppError::from)
}

pub fn list_mirrored_runs(ctx: &AppContext) -> Result<Vec<RemoteRunMirror>, AppError> {
    ctx.remote_run_mirror.list().map_err(AppError::from)
}

pub fn find_mirror_for_feature(
    ctx: &AppContext,
    feature_id: String,
) -> Result<Option<RemoteRunMirror>, AppError> {
    Ok(ctx
        .remote_run_mirror
        .list()
        .map_err(AppError::from)?
        .into_iter()
        .find(|row| row.feature_id.as_deref() == Some(feature_id.as_str())))
}

pub async fn reconcile_all_runs(
    ctx: &AppContext,
    notify: &(dyn Fn(&[String]) + Sync),
) -> Result<Vec<RemoteRunMirror>, AppError> {
    let rows = ctx.remote_run_mirror.list().map_err(AppError::from)?;
    let mut notify_bodies = Vec::new();
    for row in &rows {
        let Some((status, error)) = reconcile_one_run(ctx, row).await else {
            continue;
        };
        if NOTIFY_ON.contains(&status.as_str())
            && row.last_notified_status.as_deref() != Some(status.as_str())
        {
            let body = match status.as_str() {
                "awaiting_mr" | "completed" => format!("{} — PR ready", row.title),
                "failed" => format!(
                    "{} — failed{}",
                    row.title,
                    error
                        .as_deref()
                        .map(|error| format!(": {error}"))
                        .unwrap_or_default()
                ),
                "parked" => format!("{} — parked, needs your decision", row.title),
                "over-budget" => format!("{} — hit its budget cap", row.title),
                "needs-credentials" => {
                    format!("{} — needs credentials re-injected", row.title)
                }
                _ => row.title.clone(),
            };
            notify_bodies.push(body);
            let _ = ctx
                .remote_run_mirror
                .mark_notified(&row.machine_id, &row.run_id, &status);
        }
    }
    if !notify_bodies.is_empty() {
        notify(&notify_bodies);
    }
    ctx.remote_run_mirror.list().map_err(AppError::from)
}
