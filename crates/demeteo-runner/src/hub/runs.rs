//! Where a Hub request lands on this runner: the bodies of its own control
//! socket (decision 64, read per instance). A Hub cancel acts on any run here,
//! whichever desktop submitted it — the Hub is the control plane of the one
//! tenant this runner serves, not another client of it — so it skips the
//! owner check the socket's `cancel_run` makes, and nothing else.
//!
//! Holds [`RunnerServices`], which is why `HubControl` and its policy writer
//! are not among them.

use crate::control;
use crate::services::RunnerServices;
use async_trait::async_trait;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::error::AppError;
use demeteo_core::ports::hub::{HubRunControl, StartSupport};
use demeteo_core::ports::step_executor::FeatureLaunch;
use std::sync::Arc;

pub struct RunnerHubRuns {
    svc: Arc<RunnerServices>,
}

impl RunnerHubRuns {
    pub fn new(svc: Arc<RunnerServices>) -> Self {
        Self { svc }
    }
}

#[async_trait]
impl HubRunControl for RunnerHubRuns {
    /// A runner runs what a desktop submits with a spec, project settings and
    /// a git credential; a Hub start carries none of them.
    fn starts(&self) -> StartSupport {
        StartSupport::Refused
    }

    async fn launch(&self, _: FeatureLaunch) -> Result<FeatureId, AppError> {
        Err(AppError::validation(
            "this runner does not start runs for the Hub",
        ))
    }

    async fn cancel(&self, feature_id: &FeatureId) -> Result<(), AppError> {
        let run = control::run_for_feature(self.svc.ctx.runner_runs.as_ref(), feature_id.as_ref())?
            .ok_or_else(|| AppError::not_found(format!("no run of feature {feature_id}")))?;
        control::cancel_run(&self.svc.ctx, &self.svc.creds, run).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
