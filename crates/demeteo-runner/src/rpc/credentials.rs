use crate::services::RunnerServices;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::paths;
use demeteo_core::ports::runner_run::RunnerRun;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::ownership::require_owner;

#[derive(Debug, Deserialize)]
struct ProbeAgentParams {
    kind: String,
}

#[derive(Debug, Serialize)]
pub(super) struct ProbeAgentResult {
    kind: String,
    available: bool,
}

#[derive(Debug, Deserialize)]
struct InjectCredentialsParams {
    run_id: String,
    git_pat: String,
}

/// M4.1: lets the laptop check agent readiness *before* calling
/// `submit_run`, so a bad launch can be blocked in the UI with a clear
/// message instead of round-tripping through a rejected submission.
/// Reuses `AgentRegistry::is_available` (the same probe the desktop app's
/// settings page uses for its "Re-check agent availability" button) —
/// this is a presence/PATH check, not a full auth verification; the
/// coding agent's own auth is a machine precondition per R5/§6.1, not
/// something this runner can validate generically across agent kinds.
pub(super) async fn probe_agent(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<ProbeAgentResult, String> {
    let params: ProbeAgentParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let available = svc
        .ctx
        .registry
        .is_available(&params.kind, svc.ctx.exec.as_ref(), "local", false)
        .await;
    Ok(ProbeAgentResult {
        kind: params.kind,
        available,
    })
}

/// M4.2: push the run-scoped git-provider PAT into runner memory only.
/// Separate from `submit_run` so it can be re-supplied after a runner
/// restart loses the in-memory copy (§6.2/§7.1). If the run is currently
/// parked at `needs-credentials` (either the pre-clone or the terminal
/// push was waiting on this), this re-drives it to completion instead of
/// leaving it parked until the next poll of `get_status` (there is no
/// next poll — the run is not actively retrying on its own).
pub(super) async fn inject_credentials(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
    client_id: &str,
) -> Result<RunnerRun, String> {
    let params: InjectCredentialsParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    // MC-D2: confirm ownership *before* touching the credential store — a
    // non-owner must not be able to seed (or overwrite) another client's
    // run PAT, and gets the uniform "no such run" either way.
    let run = require_owner(svc, &params.run_id, client_id)?;
    svc.creds.insert(&params.run_id, params.git_pat);

    if run.status != "needs-credentials" {
        // Common case: this arrives right after `submit_run`, before the
        // background task even reaches its first `wait_for_pat` — nothing
        // to resume, the in-memory store already has what it needs.
        return Ok(run);
    }

    let spec: RunSpec =
        serde_json::from_str(&run.spec_json).map_err(|e| format!("unparseable spec: {}", e))?;
    let now = paths::now_ms();
    svc.ctx
        .runner_runs
        .update_status(&params.run_id, "running", None, None, None, None, now)?;

    let svc_bg = svc.clone();
    let run_id_bg = params.run_id.clone();
    let project_id = run.project_id.clone();
    let feature_id = run.feature_id.clone();
    tokio::spawn(async move {
        let result =
            crate::run::resume_or_run(&svc_bg, &run_id_bg, &spec, project_id, feature_id).await;
        let now = paths::now_ms();
        match result {
            Ok(outcome) => {
                let _ = svc_bg.ctx.runner_runs.update_status(
                    &run_id_bg,
                    &outcome.status,
                    outcome.project_id.as_deref(),
                    outcome.feature_id.as_deref(),
                    None,
                    outcome.pushed_branch.as_deref(),
                    now,
                );
            }
            Err(e) => {
                let _ = svc_bg.ctx.runner_runs.update_status(
                    &run_id_bg,
                    "failed",
                    None,
                    None,
                    Some(&e),
                    None,
                    now,
                );
            }
        }
    });

    svc.ctx
        .runner_runs
        .get(&params.run_id)?
        .ok_or_else(|| "run vanished during credential injection".to_string())
}
