//! The production [`HubRunCredentials`]: one task per Hub-started run that
//! fetches the run's GitHub App token from the Hub's `/git-token`, holds it in
//! the run's slot of the [`crate::credentials::CredentialStore`], and replaces
//! it before it expires, for as long as the run can use one (decision 71).
//! What it does on each answer is [`super::keeper_policy`]'s to decide.
//!
//! The token takes the path an injected PAT does: a driver waiting in
//! `wait_for_pat` reads it from the store, and a run already parked
//! `needs-credentials` is re-driven through `reopen_run_driver`, as
//! `inject_credentials` re-drives it. A nudge is only ever sent to a parked
//! run: one sent to a live driver is a re-arm request, which would run the
//! whole pass again after it ends.
//!
//! The token is only ever in the store and in this task's memory. It is never
//! formatted: what reaches the log is a [`StopReason`] or a status class.

use super::keeper_policy::{watch, Answer, Attempts, Step, StopReason, Timing, Watch};
use super::runs::{HubRunCredentials, HUB_CLIENT_ID};
use crate::services::RunnerServices;
use async_trait::async_trait;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::ports::hub::{GitTokenFetchError, HubGitTokenFetcher};
use demeteo_core::shared::time::now_ms;
use std::sync::Arc;

/// The run a keeper serves, and what the Hub knows it by.
#[derive(Debug, Clone)]
pub(crate) struct KeptRun {
    pub run_id: String,
    pub request_id: String,
    pub feature_id: String,
}

#[derive(Clone)]
pub(crate) struct HubGitCredentials {
    svc: Arc<RunnerServices>,
    fetcher: Arc<dyn HubGitTokenFetcher>,
    timing: Timing,
}

impl HubGitCredentials {
    pub fn new(svc: Arc<RunnerServices>, fetcher: Arc<dyn HubGitTokenFetcher>) -> Self {
        Self::with_timing(svc, fetcher, Timing::PRODUCTION)
    }

    pub fn with_timing(
        svc: Arc<RunnerServices>,
        fetcher: Arc<dyn HubGitTokenFetcher>,
        timing: Timing,
    ) -> Self {
        Self {
            svc,
            fetcher,
            timing,
        }
    }

    pub fn spawn(&self, run: KeptRun) -> tokio::task::JoinHandle<()> {
        let keeper = self.clone();
        tokio::spawn(async move { keeper.keep(run).await })
    }

    async fn keep(&self, run: KeptRun) {
        let mut attempts = Attempts::default();
        let mut until_ms = now_ms();
        loop {
            if !self.watch_until(&run.run_id, until_ms).await {
                return;
            }
            let fetched = self.fetcher.fetch(&run.request_id, &run.feature_id).await;
            let answer = match &fetched {
                Ok(token) => Answer::Token {
                    expires_at: token.expires_at,
                },
                Err(GitTokenFetchError::Refused { status }) => Answer::Refused { status: *status },
                Err(GitTokenFetchError::Unavailable(_)) => Answer::Unavailable,
            };
            let now = now_ms();
            until_ms = match (attempts.after(answer, now, &self.timing), fetched) {
                (Step::Hold { refresh_at_ms }, Ok(token)) => {
                    if !self
                        .svc
                        .creds
                        .deliver(&run.run_id, token.token.expose().to_owned())
                    {
                        return;
                    }
                    refresh_at_ms
                }
                (Step::Retry { after }, _) => {
                    now.saturating_add(u64::try_from(after.as_millis()).unwrap_or(u64::MAX))
                }
                (Step::Stop(reason), _) => return self.stop(&run.run_id, reason),
                // `Hold` is only ever decided from a token.
                (Step::Hold { .. }, Err(_)) => return,
            };
        }
    }

    fn stop(&self, run_id: &str, reason: StopReason) {
        eprintln!(
            "[demeteo-runner] run {run_id}: no more git tokens from the Hub: {}",
            reason.note()
        );
        self.svc.creds.close_delivery(run_id);
    }

    /// Reads the run's status every `status_poll` until `until_ms`. `false`
    /// when the run is over, its credential then wiped: whatever the driver
    /// did, a run past its end holds nothing. A read that fails is waited
    /// past, never taken for an end.
    async fn watch_until(&self, run_id: &str, until_ms: u64) -> bool {
        loop {
            match self.svc.ctx.runner_runs.get(run_id) {
                Ok(row) => {
                    let holds = self.svc.creds.get(run_id).is_some();
                    match watch(row.as_ref().map(|r| r.status.as_str()), holds) {
                        Watch::End => {
                            self.svc.creds.remove(run_id);
                            return false;
                        }
                        Watch::Nudge => {
                            if let Some(row) = row {
                                self.nudge(run_id, &row.spec_json);
                            }
                        }
                        Watch::Continue => {}
                    }
                }
                Err(e) => eprintln!(
                    "[demeteo-runner] run {run_id}: credential keeper could not read the run: {e}"
                ),
            }
            let now = now_ms();
            if now >= until_ms {
                return true;
            }
            let wait = (until_ms - now).min(poll_ms(&self.timing));
            tokio::time::sleep(std::time::Duration::from_millis(wait)).await;
        }
    }

    fn nudge(&self, run_id: &str, spec_json: &str) {
        let spec: RunSpec = match serde_json::from_str(spec_json) {
            Ok(spec) => spec,
            Err(e) => {
                eprintln!("[demeteo-runner] run {run_id}: unparseable spec, not resumed: {e}");
                return;
            }
        };
        if let Err(e) = crate::run::reopen_run_driver(&self.svc, run_id, spec) {
            eprintln!("[demeteo-runner] run {run_id}: not resumed with its git token: {e}");
        }
    }
}

fn poll_ms(timing: &Timing) -> u64 {
    u64::try_from(timing.status_poll.as_millis())
        .unwrap_or(u64::MAX)
        .max(1)
}

#[async_trait]
impl HubRunCredentials for HubGitCredentials {
    /// Returns as soon as the keeper is started: the first answer is a `404`
    /// until the accepted result this launch returns has reached the Hub.
    async fn deliver(
        &self,
        run_id: &str,
        request_id: &str,
        feature_id: &FeatureId,
    ) -> Result<(), String> {
        self.spawn(KeptRun {
            run_id: run_id.to_owned(),
            request_id: request_id.to_owned(),
            feature_id: feature_id.to_string(),
        });
        Ok(())
    }
}

/// The Hub runs a restart left able to use a credential, each with its
/// delivery window already open. Called before the restart reconciliation
/// starts any driver, so a driver that ends at once closes a window that is
/// already there. The request id is read back from the log of what the Hub
/// asked here, which recorded it beside the feature the start was accepted as.
pub(crate) fn hub_runs_to_keep(svc: &RunnerServices) -> Vec<KeptRun> {
    let runs = match svc.ctx.runner_runs.list() {
        Ok(runs) => runs,
        Err(e) => {
            eprintln!("[demeteo-runner] Hub runs not resumed with git tokens: {e}");
            return Vec::new();
        }
    };
    runs.into_iter()
        .filter(|run| run.owner_client_id == HUB_CLIENT_ID)
        .filter(|run| restart_keeps(&run.status))
        .filter_map(|run| {
            let feature_id = run.feature_id.clone().or_else(|| {
                serde_json::from_str::<RunSpec>(&run.spec_json)
                    .ok()
                    .and_then(|spec| spec.feature_id)
            })?;
            match svc.ctx.hub_requests.accepted_start_of(&feature_id) {
                Ok(Some(request_id)) => {
                    svc.creds.open_delivery(&run.run_id);
                    Some(KeptRun {
                        run_id: run.run_id,
                        request_id,
                        feature_id,
                    })
                }
                Ok(None) => {
                    eprintln!(
                        "[demeteo-runner] run {}: no accepted Hub start is logged for it; no git token",
                        run.run_id
                    );
                    None
                }
                Err(e) => {
                    eprintln!(
                        "[demeteo-runner] run {}: its Hub start could not be read: {e}",
                        run.run_id
                    );
                    None
                }
            }
        })
        .collect()
}

/// The statuses a restart leaves a run in that can still use a credential:
/// the ones reconciliation resumes, and a park that waits for one.
fn restart_keeps(status: &str) -> bool {
    demeteo_core::domain::reboot_budget::left_unfinished_by_restart(status)
        || status == "needs-credentials"
}

#[cfg(test)]
#[path = "keeper_tests.rs"]
mod tests;
