//! `publish_run(run_id, git_pat)`: push a finished run's branch and open its
//! PR again, from this runner's clone — the only place the branch exists. The
//! decision is [`demeteo_core::domain::runner_publish`]'s.
//!
//! Answers as soon as the publish is started: the push runs the target repo's
//! `pre-push` hook, which may run its whole test suite. The outcome arrives the
//! way every other run ending does, through [`crate::run::settle_run`] and the
//! event log.

use crate::services::RunnerServices;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::domain::runner_publish::publish_refusal;
use demeteo_core::ports::runner_run::RunnerRun;
use serde::Deserialize;
use std::sync::Arc;

use super::ownership::require_owner;

#[derive(Debug, Deserialize)]
struct PublishRunParams {
    run_id: String,
    git_pat: String,
}

pub(super) async fn publish_run(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<RunnerRun, String> {
    let params: PublishRunParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let run = require_owner(svc, &params.run_id, client_id)?;
    let (Some(_), Some(feature_id)) = (&run.project_id, run.feature_id.clone()) else {
        return Err(format!(
            "run {} never bootstrapped a feature, so it has no branch to publish",
            run.run_id
        ));
    };
    let feature = svc
        .ctx
        .features
        .get(&FeatureId::from(feature_id.clone()))?
        .ok_or_else(|| format!("run {} lost its feature {feature_id}", run.run_id))?;
    if let Some(refusal) = publish_refusal(&run.status, &feature.status, feature.mr_url.as_deref())
    {
        return Err(refusal.to_string());
    }
    let spec: RunSpec = serde_json::from_str(&run.spec_json)
        .map_err(|e| format!("run {} has an unparseable spec: {}", run.run_id, e))?;

    svc.creds.insert(&run.run_id, params.git_pat);
    crate::run::emit(&svc.ctx, &run.run_id, "publish_requested", &feature_id);
    crate::run::reopen_run_driver(svc, &run.run_id, spec)?;

    svc.ctx
        .runner_runs
        .get(&run.run_id)?
        .ok_or_else(|| "run vanished while starting its publish".to_string())
}
