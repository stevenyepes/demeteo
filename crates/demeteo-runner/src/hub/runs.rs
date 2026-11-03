//! Where a Hub request lands on this runner: the bodies of its own control
//! socket (decision 64, read per instance). A Hub cancel acts on any run here,
//! whichever desktop submitted it — the Hub is the control plane of the one
//! tenant this runner serves, not another client of it — so it skips the
//! owner check the socket's `cancel_run` makes, and nothing else. A Hub start
//! is the socket's own [`submit`], owned by [`HUB_CLIENT_ID`].
//!
//! Holds [`RunnerServices`], which is why `HubControl` and its policy writer
//! are not among them.

use crate::control;
use crate::rpc::submit;
use crate::services::RunnerServices;
use async_trait::async_trait;
use demeteo_core::application::attachments::StagedAttachmentInput;
use demeteo_core::domain::hub_request::{runner_run_spec, runner_start_refusal};
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::run_spec::RunSpecAttachment;
use demeteo_core::error::AppError;
use demeteo_core::paths;
use demeteo_core::ports::hub::{
    HubLaunch, HubLaunched, HubRunControl, RefusalReason, StartFeature, StartSupport,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The owner a Hub-started run is stamped with. The control socket refuses
/// any request naming it, so no desktop lists a Hub run or reaches one over
/// SSH, and none can pose as the Hub (docs/MULTI_CLIENT_RUNNER.md MC-D2).
pub const HUB_CLIENT_ID: &str = "hub";

/// Where a Hub-started run's git credential comes from. Composing one is what
/// lets [`RunnerHubRuns`] launch at all (decision 64, *Hub-started runner
/// runs*): without it a start would spend the Hub's only copy of its
/// attachments on a run that cannot clone.
///
/// Production composes none until the Hub mints tokens, and that token is
/// fetched over the socket after acceptance, never signed into the start
/// (decision 71). The run waits for its credential as any submitted run does,
/// and parks `needs-credentials` without one.
#[async_trait]
pub trait HubRunCredentials: Send + Sync {
    /// Called once `run_id` is submitted. An `Err` does not unwind the run;
    /// it is the accepted start's note.
    async fn deliver(&self, run_id: &str) -> Result<(), String>;
}

pub struct RunnerHubRuns {
    svc: Arc<RunnerServices>,
    credentials: Option<Arc<dyn HubRunCredentials>>,
}

impl RunnerHubRuns {
    pub fn new(svc: Arc<RunnerServices>, credentials: Option<Arc<dyn HubRunCredentials>>) -> Self {
        Self { svc, credentials }
    }

    /// The run's cap is recorded before its driver starts, so no run is ever
    /// executing uncounted; a submit that then fails zeroes it, because a
    /// refused start committed nothing.
    async fn submit_start(
        &self,
        HubLaunch { start, launch }: HubLaunch,
        run_id: &str,
        spool: &Path,
    ) -> Result<FeatureId, AppError> {
        let runner = start
            .runner
            .as_deref()
            .ok_or_else(|| AppError::validation("a Hub start to a runner has no runner part"))?;
        let feature_id = format!("f-{}", paths::new_id());
        let attachments = spool_attachments(spool, launch.staged_attachments)?;
        let spec = runner_run_spec(&start, runner, &feature_id, attachments)
            .map_err(|reason| AppError::validation(format!("Hub start refused: {reason:?}")))?;
        let spec_json = serde_json::to_string(&spec).map_err(|e| e.to_string())?;
        let runs = self.svc.ctx.runner_runs.as_ref();
        runs.get_or_create(run_id, &spec_json, HUB_CLIENT_ID, paths::now_ms())?;
        if let Err(e) = runs.record_hub_commitment(run_id, start.max_budget_cents.unwrap_or(0)) {
            runs.update_status(
                run_id,
                "failed",
                None,
                None,
                Some(&e),
                None,
                paths::now_ms(),
            )?;
            return Err(e.into());
        }
        let run = submit(&self.svc, run_id, spec, HUB_CLIENT_ID).await?;
        if run.status == "failed" {
            if let Err(e) = runs.record_hub_commitment(run_id, 0) {
                eprintln!(
                    "[demeteo-runner] warning: run {run_id} keeps the cap of a refused start: {e}"
                );
            }
            return Err(AppError::validation(
                run.error
                    .unwrap_or_else(|| format!("run {run_id} failed at submit")),
            ));
        }
        Ok(FeatureId::from(feature_id))
    }
}

#[async_trait]
impl HubRunControl for RunnerHubRuns {
    fn starts(&self) -> StartSupport {
        match self.credentials {
            Some(_) => StartSupport::Launches,
            None => StartSupport::Refused,
        }
    }

    fn committed_cents_since(&self, since_ms: i64) -> Result<u64, String> {
        self.svc.ctx.runner_runs.hub_committed_cents_since(since_ms)
    }

    fn resolve(&self, start: &StartFeature) -> Result<(), RefusalReason> {
        runner_start_refusal(start).map_or(Ok(()), Err)
    }

    /// The spool is the run's from a successful submit on, and its driver
    /// deletes it with the run's credential; before that it is deleted here.
    async fn launch(&self, launch: HubLaunch) -> Result<HubLaunched, AppError> {
        let Some(credentials) = &self.credentials else {
            return Err(AppError::validation(
                "this runner has no git credential for a Hub-started run",
            ));
        };
        let run_id = format!("hub-{}", paths::new_id());
        let spool = spool_dir(&self.svc.ctx.app_data_dir, &run_id);
        let feature_id = match self.submit_start(launch, &run_id, &spool).await {
            Ok(feature_id) => feature_id,
            Err(e) => {
                remove_spool(&spool);
                return Err(e);
            }
        };
        let note = credentials
            .deliver(&run_id)
            .await
            .err()
            .map(|e| format!("git credential not delivered: {e}"));
        Ok(HubLaunched { feature_id, note })
    }

    async fn cancel(&self, feature_id: &FeatureId) -> Result<(), AppError> {
        let run = control::run_for_feature(self.svc.ctx.runner_runs.as_ref(), feature_id.as_ref())?
            .ok_or_else(|| AppError::not_found(format!("no run of feature {feature_id}")))?;
        control::cancel_run(&self.svc.ctx, &self.svc.creds, run).await?;
        Ok(())
    }
}

/// The per-run spool a desktop's SFTP writes a detached run's attachments to
/// (`application/remote_runs/attachments.rs`), under this runner's own data
/// directory. Its last component is the run id, which is what lets the run's
/// driver delete it (`cleanup_attachment_spool` in `run.rs`).
fn spool_dir(data_dir: &Path, run_id: &str) -> PathBuf {
    data_dir.join("attachment-spool").join(run_id)
}

/// Each file is named by its index and the last component of its sniffed
/// name, so a name the Hub's bytes suggested can never leave the spool.
fn spool_attachments(
    spool: &Path,
    staged: Vec<StagedAttachmentInput>,
) -> Result<Vec<RunSpecAttachment>, AppError> {
    if staged.is_empty() {
        return Ok(Vec::new());
    }
    std::fs::create_dir_all(spool).map_err(|e| e.to_string())?;
    staged
        .into_iter()
        .enumerate()
        .map(|(i, attachment)| {
            let bytes = attachment.bytes.ok_or_else(|| {
                AppError::validation("a Hub attachment arrived without its bytes")
            })?;
            let name = attachment
                .source_filename
                .as_deref()
                .and_then(|name| Path::new(name).file_name())
                .and_then(|name| name.to_str())
                .unwrap_or("attachment");
            let path = spool.join(format!("{i}-{name}"));
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
            Ok(RunSpecAttachment {
                staged_path: path.to_string_lossy().into_owned(),
                mime: attachment.mime,
                source_filename: attachment.source_filename,
            })
        })
        .collect()
}

fn remove_spool(spool: &Path) {
    match std::fs::remove_dir_all(spool) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => eprintln!(
            "[demeteo-runner] failed to delete attachment spool {}: {e}",
            spool.display()
        ),
        _ => {}
    }
}

#[cfg(test)]
#[path = "runs_tests.rs"]
mod tests;
