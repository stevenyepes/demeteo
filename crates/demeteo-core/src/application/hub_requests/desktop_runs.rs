//! A desktop's [`HubRunControl`]: its launch button and its stop button.

use std::sync::Arc;

use async_trait::async_trait;
use demeteo_hub_protocol::{RefusalReason, StartFeature};

use crate::application::launch::{launch_run, LaunchRequest};
use crate::application::step_control;
use crate::domain::hub_request::desktop_start_refusal;
use crate::domain::ids::{FeatureId, ProjectId, WorkflowId};
use crate::domain::run_placement::{DetachedOptions, RunPlacement};
use crate::error::AppError;
use crate::ports::hub::{HubLaunch, HubLaunched, HubRunControl, StartSupport};
use crate::state::AppContext;

/// A Hub-started run is placed [`RunPlacement::Local`], as a run the launch
/// button starts with nothing chosen: a Hub start names no machine, and
/// nothing here asks which transport carries it.
pub struct DesktopRuns {
    ctx: Arc<AppContext>,
}

impl DesktopRuns {
    pub fn new(ctx: Arc<AppContext>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl HubRunControl for DesktopRuns {
    fn starts(&self) -> StartSupport {
        StartSupport::Launches
    }

    /// The Feature rows Hub launches wrote: a local launch writes its row
    /// before it returns, so an accepted start is always counted.
    fn committed_cents_since(&self, since_ms: i64) -> Result<u64, String> {
        self.ctx.features.hub_committed_cents_since(since_ms)
    }

    /// A failed lookup is `MalformedBody`, since `RefusalReason` has no
    /// generic failure, with the detail kept to this desktop's own log.
    fn resolve(&self, start: &StartFeature) -> Result<(), RefusalReason> {
        if let Some(reason) = desktop_start_refusal(start) {
            return Err(reason);
        }
        let project = ProjectId::from(start.project_id.clone());
        match self.ctx.projects.get_project(&project) {
            Ok(Some(_)) => {}
            Ok(None) => return Err(RefusalReason::UnknownProject),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the project lookup failed");
                return Err(RefusalReason::MalformedBody);
            }
        }
        let workflow = WorkflowId::from(start.workflow_id.clone());
        match self.ctx.workflows.get(&workflow) {
            Ok(Some(_)) => Ok(()),
            Ok(None) => Err(RefusalReason::UnknownWorkflow),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the workflow lookup failed");
                Err(RefusalReason::MalformedBody)
            }
        }
    }

    /// A local run has no wall-clock cap to give it: `check_options` refuses
    /// one for [`RunPlacement::Local`], so it is not passed on, and the
    /// accepted start says so.
    async fn launch(&self, launch: HubLaunch) -> Result<HubLaunched, AppError> {
        let note = launch
            .start
            .max_wall_clock_secs
            .map(|_| "max_wall_clock_secs ignored".to_owned());
        let run = launch_run(
            &self.ctx,
            LaunchRequest {
                launch: launch.launch,
                placement: RunPlacement::Local,
                detached: DetachedOptions::default(),
            },
        )
        .await?;
        Ok(HubLaunched {
            feature_id: run.feature.id,
            note,
        })
    }

    async fn cancel(&self, feature_id: &FeatureId) -> Result<(), AppError> {
        step_control::cancel_feature(&self.ctx, feature_id).await
    }
}
