//! Acting on a run that has already started, or on one of its steps,
//! wherever it runs.
//!
//! A run the desktop drives is reached through the executor ports; a run a
//! `demeteo-runner` owns is reached through
//! [`remote_runs`](crate::application::remote_runs), and the local executor
//! refuses it ([`shadow_refusal`](crate::domain::run_control::shadow_refusal)).
//! Which of the two a run belongs to is decided here, once, so a caller names
//! a feature or a step and nothing else — the role
//! [`launch_run`](crate::application::launch::launch_run) has for launches.

use crate::application::remote_runs::{
    cancel_remote_run, decide_gate, find_mirror_for_feature, retry_remote_step,
    set_remote_step_assignment, RemoteRewind, RewindOverrides,
};
use crate::domain::ids::{FeatureId, StepExecutionId};
use crate::domain::step_assignment::StepAssignment;
use crate::error::AppError;
use crate::state::AppContext;

enum Owner {
    Desktop,
    Runner { machine_id: String, run_id: String },
}

fn feature_owner(ctx: &AppContext, feature_id: &FeatureId) -> Result<Owner, AppError> {
    Ok(
        match find_mirror_for_feature(ctx, feature_id.as_ref().to_string())? {
            Some(mirror) => Owner::Runner {
                machine_id: mirror.machine_id,
                run_id: mirror.run_id,
            },
            None => Owner::Desktop,
        },
    )
}

fn owner_of(ctx: &AppContext, step_execution_id: &StepExecutionId) -> Result<Owner, AppError> {
    let step = ctx
        .features
        .step_get(step_execution_id)
        .map_err(AppError::from)?
        .ok_or_else(|| AppError::not_found("step execution not found"))?;
    feature_owner(ctx, &step.feature_id)
}

/// Stop a run, feature-wide: the desktop's stop button and a Hub cancel both
/// call this. A run a `demeteo-runner` owns is cancelled on the runner,
/// since the local executor refuses it.
pub async fn cancel_feature(ctx: &AppContext, feature_id: &FeatureId) -> Result<(), AppError> {
    match feature_owner(ctx, feature_id)? {
        Owner::Desktop => ctx
            .executor
            .feature_cancel(feature_id.as_ref())
            .await
            .map_err(AppError::from),
        Owner::Runner { machine_id, run_id } => cancel_remote_run(ctx, machine_id, run_id).await,
    }
}

/// Re-run one step that failed, was interrupted, or never started. The
/// feature-wide agent, model and effort are left as they are.
pub async fn retry_step(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
) -> Result<(), AppError> {
    match owner_of(ctx, step_execution_id)? {
        Owner::Desktop => {
            ctx.executor
                .step_retry(step_execution_id.as_ref(), None, None, None)
                .await
        }
        Owner::Runner { machine_id, run_id } => {
            retry_remote_step(
                ctx,
                machine_id,
                run_id,
                step_execution_id.as_ref().to_string(),
                RewindOverrides::default(),
                RemoteRewind::Retry,
            )
            .await
        }
    }
}

/// Pin which agent, model and effort one step runs as.
pub async fn set_step_assignment(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
    assignment: StepAssignment,
) -> Result<(), AppError> {
    match owner_of(ctx, step_execution_id)? {
        Owner::Desktop => {
            ctx.executor
                .step_set_assignment(step_execution_id.as_ref(), assignment)
                .await
        }
        Owner::Runner { machine_id, run_id } => {
            set_remote_step_assignment(
                ctx,
                machine_id,
                run_id,
                step_execution_id.as_ref().to_string(),
                assignment,
            )
            .await
        }
    }
}

/// Send a parked gate back with `feedback`.
///
/// `redirect` is the only decision this spells. Approval has no function here
/// on purpose: the callers of this module include surfaces that must never be
/// able to approve (`docs/MCP_INTEGRATION.md` §8).
pub async fn redirect_gate(
    ctx: &AppContext,
    step_execution_id: &StepExecutionId,
    feedback: &str,
) -> Result<(), AppError> {
    match owner_of(ctx, step_execution_id)? {
        Owner::Desktop => {
            ctx.presenter
                .gate_decide(step_execution_id.as_ref(), "redirect", Some(feedback))
                .await
        }
        Owner::Runner { machine_id, run_id } => {
            decide_gate(
                ctx,
                machine_id,
                run_id,
                step_execution_id.as_ref().to_string(),
                "redirect".to_string(),
                Some(feedback.to_string()),
            )
            .await
        }
    }
}
