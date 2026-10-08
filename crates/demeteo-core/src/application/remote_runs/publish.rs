//! Publish MR for a feature, from wherever its branch lives.
//!
//! A detached run's branch exists only in the runner's clone. Publishing it
//! through [`MrPublisher`](crate::ports::mr_publisher::MrPublisher) pushed from
//! the desktop's clone instead, which never had the branch, and git refused
//! with `src refspec … does not match any`. So a feature a runner holds is
//! published by that runner — see [`crate::domain::runner_publish`].

use serde::Serialize;

use super::control::find_mirror_for_feature;
use super::credentials::pat_for_run;
use super::reconcile::reconcile_one_run;
use super::rpc::{json_str, remote_rpc};
use crate::domain::ids::FeatureId;
use crate::domain::models::{MrInfo, PublishOptions};
use crate::domain::runner_publish::METHOD;
use crate::error::AppError;
use crate::state::AppContext;

/// What a Publish MR did.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PublishOutcome {
    /// The PR is open, or already was.
    Opened { mr: MrInfo },
    /// The runner holding the run started its push; the run's event log
    /// carries the result, because the push runs the repo's `pre-push` hook
    /// and may take as long as its test suite.
    OnRunner { machine_id: String, run_id: String },
}

pub async fn publish_feature(
    ctx: &AppContext,
    project_id: &str,
    feature_id: &FeatureId,
    options: PublishOptions,
) -> Result<PublishOutcome, AppError> {
    let has_pr = ctx
        .features
        .get(feature_id)
        .map_err(AppError::from)?
        .and_then(|f| f.mr_url)
        .is_some_and(|url| !url.is_empty());
    let mirror = if has_pr {
        None
    } else {
        find_mirror_for_feature(ctx, feature_id.0.clone())?
    };
    let Some(row) = mirror else {
        let mr = ctx
            .mr_publisher
            .publish_mr(project_id, feature_id, options)
            .await
            .map_err(AppError::from)?;
        return Ok(PublishOutcome::Opened { mr });
    };

    let pat = pat_for_run(ctx, &row)?;
    let answer = remote_rpc(
        ctx,
        &row.machine_id,
        METHOD,
        serde_json::json!({ "run_id": row.run_id, "git_pat": pat }),
    )
    .await
    .map_err(AppError::from)?;
    let status = json_str(&answer, "status").unwrap_or_else(|| "running".to_string());
    ctx.remote_run_mirror
        .update_status(
            &row.machine_id,
            &row.run_id,
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
    Ok(PublishOutcome::OnRunner {
        machine_id: row.machine_id,
        run_id: row.run_id,
    })
}
