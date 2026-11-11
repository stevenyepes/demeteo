//! A Hub `retry_step` once its id is spent (docs/HUB.md §6.1): the step it
//! names resolved and judged, then handed to the instance's
//! [`HubRunControl::retry`](crate::ports::hub::HubRunControl::retry).

use demeteo_hub_protocol::{RefusalReason, RetryStep};

use super::{HubRequests, Settled};
use crate::application::remote_runs::find_mirror_for_feature;
use crate::domain::hub_request::{hub_retry_refusal, latest_execution, retry_call_refusal};
use crate::domain::ids::FeatureId;

impl HubRequests {
    /// Refused in the order a gate decision is (HUB.md §8): the Feature, the
    /// step, then `runner_owned` before the step's status, because the
    /// status of a run a runner owns is the desktop's mirror of it and may
    /// lag the runner's own. A paired runner answers for its own runs; a
    /// desktop forwarding this one would retry it on a report.
    pub(super) async fn retry(&self, retry: &RetryStep) -> Settled {
        let feature_id = FeatureId::from(retry.feature_id.clone());
        if let Err(settled) = self.known_feature(&feature_id) {
            return settled;
        }
        let steps = match self.ctx.features.steps_for_feature(&feature_id) {
            Ok(steps) => steps,
            Err(e) => {
                tracing::warn!(error = %e, "Hub retry refused: the steps did not load");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        };
        let Some(step) = latest_execution(&steps, &retry.step_id) else {
            return Settled::refused(RefusalReason::UnknownStep);
        };
        match find_mirror_for_feature(&self.ctx, retry.feature_id.clone()) {
            Ok(None) => {}
            Ok(Some(_)) => return Settled::refused(RefusalReason::RunnerOwned),
            Err(e) => {
                tracing::warn!(error = %e, "Hub retry refused: the run's owner did not load");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        }
        if let Some(reason) = hub_retry_refusal(step) {
            return Settled::refused(reason);
        }
        match self.runs.retry(&feature_id, &step.id).await {
            Ok(()) => Settled::accepted(Some(retry.feature_id.clone()), None),
            Err(e) => {
                tracing::warn!(error = %e, "Hub retry refused: the retry failed");
                Settled::refused(retry_call_refusal(&e))
            }
        }
    }
}
