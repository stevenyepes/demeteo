//! A Hub start, once its id is spent: the lookups, the attachment fetch
//! and the launch. What is decided is decided in
//! [`crate::domain::hub_request`].

use demeteo_hub_protocol::{RefusalReason, StartFeature};

use super::{HubRequests, Settled};
use crate::adapters::step_executor::setup::slug_from_description;
use crate::application::attachments::{
    StagedAttachmentInput, MAX_AGENT_ATTACHMENT_BATCH_BYTES, MAX_ATTACHMENTS_PER_FEATURE,
    MAX_ATTACHMENT_BYTES,
};
use crate::application::launch::{launch_run, LaunchRequest};
use crate::domain::attachment::{sniff_attachment_mime, sniffed_source_filename};
use crate::domain::feature_origin::FeatureOrigin;
use crate::domain::hub_request::log::launch_failure_reason;
use crate::domain::hub_request::{
    attachment_refusal, effort_level, start_refusal, step_override, AttachmentLimits,
};
use crate::domain::ids::{ProjectId, WorkflowId};
use crate::domain::models::LaunchedBy;
use crate::domain::run_placement::{DetachedOptions, RunPlacement};
use crate::ports::hub::AttachmentFetchError;
use crate::ports::step_executor::FeatureLaunch;
use crate::shared::secret_scrub::scrub_secrets;

const ATTACHMENT_LIMITS: AttachmentLimits = AttachmentLimits {
    max_count: MAX_ATTACHMENTS_PER_FEATURE,
    max_bytes: MAX_ATTACHMENT_BYTES,
    max_total_bytes: MAX_AGENT_ATTACHMENT_BATCH_BYTES,
};

/// The forms a launch error can quote the start's text in besides verbatim:
/// the branch slug, as the executor derives it. A text with nothing to slug
/// gets the `"feature"` fallback, which is not the request's and is left
/// alone, or every `demeteo/features/…` branch in a detail would be cut.
fn withheld_slugs(start: &StartFeature) -> Vec<String> {
    [&start.title, &start.description]
        .into_iter()
        .map(|text| slug_from_description(text))
        .filter(|slug| slug != "feature")
        .collect()
}

/// The run a start asks for. `source_path` stays empty: a path named by the
/// Hub would be a path on this disk the Hub chose, so attachments arrive as
/// the bytes fetched for them and the type sniffed from those bytes: with no
/// type, the bootstrap's commit resolves `application/octet-stream` and fails.
fn feature_launch(
    start: &StartFeature,
    attachments: Vec<(Vec<u8>, &'static str)>,
) -> FeatureLaunch {
    FeatureLaunch {
        project_id: start.project_id.clone(),
        workflow_id: start.workflow_id.clone(),
        title: start.title.clone(),
        description: start.description.clone(),
        agent_kind: start.agent_kind.clone(),
        model: start.model.clone(),
        effort: start.effort.map(effort_level),
        max_budget_usd: start.max_budget_cents.map(|cents| cents as f64 / 100.0),
        step_overrides: start.step_overrides.iter().map(step_override).collect(),
        staged_attachments: attachments
            .into_iter()
            .map(|(bytes, mime)| StagedAttachmentInput {
                source_path: String::new(),
                mime: Some(mime.to_owned()),
                source_filename: sniffed_source_filename(mime, &bytes),
                bytes: Some(bytes),
            })
            .collect(),
        origin: FeatureOrigin::default(),
        launched_by: LaunchedBy::Hub,
        ..FeatureLaunch::default()
    }
}

impl HubRequests {
    /// Every check that could refuse has passed and the id is spent, so a
    /// failure from here on is answered rather than left for a redelivery
    /// that would read as `Replayed`. `RefusalReason` has no generic failure,
    /// so one that is not a lookup or an attachment is `MalformedBody`, with
    /// the detail kept to this desktop's own log.
    pub(super) async fn start(&self, start: &StartFeature) -> Settled {
        match self
            .ctx
            .projects
            .get_project(&ProjectId::from(start.project_id.clone()))
        {
            Ok(Some(_)) => {}
            Ok(None) => return Settled::refused(RefusalReason::UnknownProject),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the project lookup failed");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        }
        match self
            .ctx
            .workflows
            .get(&WorkflowId::from(start.workflow_id.clone()))
        {
            Ok(Some(_)) => {}
            Ok(None) => return Settled::refused(RefusalReason::UnknownWorkflow),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the workflow lookup failed");
                return Settled::refused(RefusalReason::MalformedBody);
            }
        }
        if let Some(reason) = start_refusal(start, &self.registered()) {
            return Settled::refused(reason);
        }
        if let Some(reason) = attachment_refusal(&start.attachments, ATTACHMENT_LIMITS) {
            return Settled::refused(reason);
        }
        let fetched = match tokio::time::timeout(self.fetch_deadline, self.fetch_all(start)).await {
            Ok(Ok(fetched)) => fetched,
            Ok(Err(e)) => {
                if e != AttachmentFetchError::Mismatch {
                    tracing::warn!(error = %e, "Hub start refused: an attachment did not arrive");
                }
                return Settled::refused(RefusalReason::AttachmentMismatch);
            }
            Err(_) => {
                tracing::warn!("Hub start refused: its attachments overran the fetch deadline");
                return Settled::refused(RefusalReason::AttachmentMismatch);
            }
        };
        // Refused here, not left to the bootstrap: the Hub has already deleted
        // the bytes, so an accept would be a run that fails before its first
        // step with nothing left to retry from.
        let mut typed = Vec::with_capacity(fetched.len());
        for bytes in fetched {
            match sniff_attachment_mime(&bytes) {
                Some(mime) => typed.push((bytes, mime)),
                None => return Settled::refused(RefusalReason::AttachmentMismatch),
            }
        }
        // A local run has no wall-clock cap to give it: `check_options`
        // refuses one for `RunPlacement::Local`, so it is not passed on.
        let note = start
            .max_wall_clock_secs
            .map(|_| "max_wall_clock_secs ignored".to_owned());
        let launched = launch_run(
            &self.ctx,
            LaunchRequest {
                launch: feature_launch(start, typed),
                placement: RunPlacement::Local,
                detached: DetachedOptions::default(),
            },
        )
        .await;
        match launched {
            Ok(run) => Settled::accepted(Some(run.feature.id.to_string()), note),
            Err(e) => {
                tracing::warn!(error = %e, "Hub start refused: the launch failed");
                let detail = scrub_secrets(&e.to_string()).into_owned();
                let slugs = withheld_slugs(start);
                let mut withheld = vec![start.title.as_str(), start.description.as_str()];
                withheld.extend(slugs.iter().map(String::as_str));
                Settled {
                    reason: Some(launch_failure_reason(&detail, &withheld)),
                    ..Settled::refused(RefusalReason::MalformedBody)
                }
            }
        }
    }

    async fn fetch_all(&self, start: &StartFeature) -> Result<Vec<Vec<u8>>, AttachmentFetchError> {
        let mut fetched = Vec::with_capacity(start.attachments.len());
        for attachment in &start.attachments {
            fetched.push(self.ports.attachments.fetch(attachment).await?);
        }
        Ok(fetched)
    }
}
