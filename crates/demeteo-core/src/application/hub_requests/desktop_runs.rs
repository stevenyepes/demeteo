//! A desktop's [`HubRunControl`]: its launch button and its stop button.

use std::sync::Arc;

use async_trait::async_trait;

use crate::application::launch::{launch_run, LaunchRequest};
use crate::application::step_control;
use crate::domain::ids::FeatureId;
use crate::domain::run_placement::{DetachedOptions, RunPlacement};
use crate::error::AppError;
use crate::ports::hub::{HubRunControl, StartSupport};
use crate::ports::step_executor::FeatureLaunch;
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

    async fn launch(&self, launch: FeatureLaunch) -> Result<FeatureId, AppError> {
        let run = launch_run(
            &self.ctx,
            LaunchRequest {
                launch,
                placement: RunPlacement::Local,
                detached: DetachedOptions::default(),
            },
        )
        .await?;
        Ok(run.feature.id)
    }

    async fn cancel(&self, feature_id: &FeatureId) -> Result<(), AppError> {
        step_control::cancel_feature(&self.ctx, feature_id).await
    }
}
