//! Control RPC server (docs/REMOTE_EXECUTION_PLAN.md M3.1/M3.2/M4/M5).
//!
//! Listens on a Unix-domain socket at `<data_dir>/control.sock` with `0600`
//! permissions. Per the design doc's decided fork (REMOTE_EXECUTION_PLAN.md
//! M3.1): protection comes from OS file permissions alone — no bearer
//! token, no listening TCP port. The laptop reaches this socket by
//! forwarding it over SSH (`ssh -L <local>:<remote.sock>`); a second local
//! user on the runner host cannot open a `0600` socket owned by a
//! different uid, so it is safe without additional authz.
//!
//! Wire format: newline-delimited JSON. One request per line in, one
//! response per line out — simple enough to test with `nc -U` or a raw
//! socket client, no RPC framework dependency.

use crate::services::RunnerServices;
use demeteo_core::domain::ids::FeatureId;
use demeteo_core::domain::run_spec::RunSpec;
use demeteo_core::paths;
use demeteo_core::ports::run_events::RunEvent;
use demeteo_core::ports::runner_run::RunnerRun;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};

pub fn socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join("control.sock")
}

#[derive(Debug, Deserialize)]
struct Request {
    id: u64,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct SubmitRunParams {
    run_id: String,
    spec: RunSpec,
}

#[derive(Debug, Deserialize)]
struct RunIdParams {
    run_id: String,
}

#[derive(Debug, Deserialize)]
struct StreamEventsParams {
    run_id: String,
    #[serde(default)]
    from_offset: i64,
}

#[derive(Debug, Deserialize)]
struct ProbeAgentParams {
    kind: String,
}

#[derive(Debug, Serialize)]
struct ProbeAgentResult {
    kind: String,
    available: bool,
}

#[derive(Debug, Deserialize)]
struct InjectCredentialsParams {
    run_id: String,
    git_pat: String,
}

#[derive(Debug, Deserialize)]
struct DecideGateParams {
    /// The RPC surface is `decide_gate(run_id, gate_id, decision)` (M5.3);
    /// `run_id` isn't needed to apply the decision (`gate_id` — the
    /// step_execution_id — is already globally unique) but is accepted
    /// so a future multi-run laptop can validate the pairing without a
    /// wire-format change.
    #[allow(dead_code)]
    run_id: String,
    gate_id: String,
    decision: String,
    #[serde(default)]
    feedback: Option<String>,
}

#[derive(Debug, Serialize)]
struct Response {
    id: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct HealthInfo {
    version: &'static str,
    pid: u32,
}

/// Bind the control socket and serve connections until the process exits.
/// Removes a stale socket file from a previous (crashed) run before
/// binding — `UnixListener::bind` fails with `AddrInUse` on an existing
/// path even if nothing is listening on it anymore.
pub async fn serve(svc: Arc<RunnerServices>, path: PathBuf) -> std::io::Result<()> {
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    // `0600`: owner read/write only. This is the entire authz model
    // (M3.1) — no other local user can open this path.
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    eprintln!("[demeteo-runner] listening on {}", path.display());

    loop {
        let (stream, _addr) = listener.accept().await?;
        let svc = svc.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(svc, stream).await {
                eprintln!("[demeteo-runner] connection error: {}", e);
            }
        });
    }
}

async fn handle_connection(svc: Arc<RunnerServices>, stream: UnixStream) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(req) => dispatch(&svc, req).await,
            Err(e) => Response {
                id: 0,
                result: None,
                error: Some(format!("invalid request: {}", e)),
            },
        };
        let mut out = serde_json::to_string(&response).unwrap_or_else(|e| {
            format!(
                r#"{{"id":0,"error":"failed to serialize response: {}"}}"#,
                e
            )
        });
        out.push('\n');
        write_half.write_all(out.as_bytes()).await?;
    }
    Ok(())
}

async fn dispatch(svc: &Arc<RunnerServices>, req: Request) -> Response {
    let result = match req.method.as_str() {
        "health" => Ok(serde_json::to_value(HealthInfo {
            version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
        })
        .unwrap()),
        "submit_run" => submit_run(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "probe_agent" => probe_agent(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "inject_credentials" => inject_credentials(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "decide_gate" => decide_gate(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "get_status" => get_status(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "list_runs" => svc
            .ctx
            .runner_runs
            .list()
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "stream_events" => stream_events(svc, req.params)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "cancel_run" => cancel_run(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        other => Err(format!("unknown method: {}", other)),
    };
    match result {
        Ok(v) => Response {
            id: req.id,
            result: Some(v),
            error: None,
        },
        Err(e) => Response {
            id: req.id,
            result: None,
            error: Some(e),
        },
    }
}

/// Idempotent by `run_id` (R9/M3.2): re-submitting the same `run_id`
/// returns the existing row instead of starting a second feature. A
/// freshly-created row is handed off to `crate::run::execute_run` on a
/// spawned task and this returns immediately — `submit_run` reports
/// "accepted", not "finished"; the caller polls `get_status`/`list_runs`
/// (or, from M3.3 on, tails the event log).
async fn submit_run(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<RunnerRun, String> {
    let params: SubmitRunParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let spec_json = serde_json::to_string(&params.spec).map_err(|e| e.to_string())?;
    let now = paths::now_ms();
    let run = svc
        .ctx
        .runner_runs
        .get_or_create(&params.run_id, &spec_json, now)?;

    if run.status != "pending" {
        // Already submitted (possibly already finished) — no-op.
        return Ok(run);
    }

    // M4.1: agent-readiness precondition. Fail loud at launch rather than
    // mid-run — a machine missing the selected agent binary is ineligible
    // for this run entirely.
    if let Some(kind) = params.spec.agent_kind.as_deref() {
        if !svc
            .ctx
            .registry
            .is_available(kind, svc.ctx.exec.as_ref(), "local", false)
            .await
        {
            let msg = format!(
                "agent '{}' is not installed/available on this machine — run rejected",
                kind
            );
            svc.ctx.runner_runs.update_status(
                &params.run_id,
                "failed",
                None,
                None,
                Some(&msg),
                None,
                now,
            )?;
            return svc
                .ctx
                .runner_runs
                .get(&params.run_id)?
                .ok_or_else(|| "run vanished immediately after creation".to_string());
        }
    }

    svc.ctx
        .runner_runs
        .update_status(&params.run_id, "running", None, None, None, None, now)?;

    let svc_bg = svc.clone();
    let run_id_bg = params.run_id.clone();
    tokio::spawn(async move {
        let result = crate::run::execute_run(&svc_bg, &run_id_bg, &params.spec).await;
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
                let _ = svc_bg.ctx.run_events.append(
                    &run_id_bg,
                    "failed",
                    serde_json::to_string(&e).ok().as_deref(),
                    now,
                );
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
        .ok_or_else(|| "run vanished immediately after creation".to_string())
}

/// M4.1: lets the laptop check agent readiness *before* calling
/// `submit_run`, so a bad launch can be blocked in the UI with a clear
/// message instead of round-tripping through a rejected submission.
/// Reuses `AgentRegistry::is_available` (the same probe the desktop app's
/// settings page uses for its "Re-check agent availability" button) —
/// this is a presence/PATH check, not a full auth verification; the
/// coding agent's own auth is a machine precondition per R5/§6.1, not
/// something this runner can validate generically across agent kinds.
async fn probe_agent(
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
async fn inject_credentials(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<RunnerRun, String> {
    let params: InjectCredentialsParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    svc.creds.insert(&params.run_id, params.git_pat);

    let run = svc
        .ctx
        .runner_runs
        .get(&params.run_id)?
        .ok_or_else(|| format!("no such run: {}", params.run_id))?;

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

/// M5.3: clear a gate the unattended policy parked (`gate_class:
/// "dangerous"`, M5.1) from the laptop. Delegates straight to
/// `GatePresenter::gate_decide` — the same application-level call the
/// desktop app's `gate_decide` Tauri command uses — so there is exactly
/// one gate-decision code path regardless of which side of the tunnel
/// the decision came from.
async fn decide_gate(svc: &Arc<RunnerServices>, params: serde_json::Value) -> Result<(), String> {
    let params: DecideGateParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    svc.ctx
        .presenter
        .gate_decide(
            &params.gate_id,
            &params.decision,
            params.feedback.as_deref(),
        )
        .await
        .map_err(|e| e.to_string())
}

/// `RunnerRun` plus a couple of fields the return inbox (M6.2) needs
/// that `RunnerRun` alone doesn't carry, since they live on the
/// runner's own `features`/gate state, not the coarse run-status column:
///
/// - `mr_url` — the PR/MR URL, once one exists, to deep-link "PR ready".
/// - `parked_gate_id` — set when a *dangerous* gate is currently parked
///   awaiting a human (M5.1). Unlike `over-budget`/`needs-credentials`,
///   a parked gate doesn't change `RunnerRun.status` (the feature is
///   still nominally "running", just blocked on this one decision), so
///   without this field the inbox can't distinguish "parked, needs you"
///   from "running fine" for an unattended run.
#[derive(Debug, Serialize)]
struct RunStatusView {
    #[serde(flatten)]
    run: RunnerRun,
    mr_url: Option<String>,
    parked_gate_id: Option<String>,
}

async fn get_status(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<RunStatusView, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let run = svc
        .ctx
        .runner_runs
        .get(&params.run_id)?
        .ok_or_else(|| format!("no such run: {}", params.run_id))?;

    let mut mr_url = None;
    let mut parked_gate_id = None;
    if let Some(fid) = run.feature_id.as_ref() {
        if let Ok(Some(feature)) = svc.ctx.features.get(&FeatureId::from(fid.clone())) {
            mr_url = feature.mr_url.clone();
            if let Ok(Some(gate_dec)) = svc.ctx.presenter.gate_pending_for_run(fid).await {
                if crate::run::gate_is_dangerous(&svc.ctx, &feature, &gate_dec) {
                    parked_gate_id = Some(gate_dec.step_execution_id.as_str().to_string());
                }
            }
        }
    }
    Ok(RunStatusView {
        run,
        mr_url,
        parked_gate_id,
    })
}

/// R9: "catch up on everything missed by offset — never relies on a live
/// socket having been connected." A client (or a laptop mirror, once
/// M6's SSH-forwarding client exists) calls this repeatedly with the
/// highest offset it's already seen; a dropped connection just means the
/// next call's `from_offset` is a little further behind, not a gap.
fn stream_events(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<Vec<RunEvent>, String> {
    let params: StreamEventsParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    svc.ctx
        .run_events
        .list_since(&params.run_id, params.from_offset)
}

/// R8: cancellation is explicit and RPC-only — closing the laptop or
/// dropping the SSH tunnel must never cancel a run. Delegates to the
/// same `StepExecutor::feature_cancel` the desktop app's `feature_cancel`
/// Tauri command already uses; no separate cancellation logic to drift.
async fn cancel_run(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<RunnerRun, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let run = svc
        .ctx
        .runner_runs
        .get(&params.run_id)?
        .ok_or_else(|| format!("no such run: {}", params.run_id))?;

    if let Some(feature_id) = &run.feature_id {
        svc.ctx
            .executor
            .feature_cancel(feature_id)
            .await
            .map_err(|e| format!("failed to cancel feature: {}", e))?;
    }

    // Atomic conditional update (not read-then-write): if the run
    // finished for real between our `get` above and this call — e.g. the
    // background execute_run task raced us to a genuine `awaiting_mr` —
    // this leaves that real outcome alone instead of stomping it to
    // `cancelled`. Returns whatever the row's true status ends up being.
    let now = paths::now_ms();
    let run = svc
        .ctx
        .runner_runs
        .cancel_if_active(&params.run_id, now)?
        .ok_or_else(|| "run vanished during cancel".to_string())?;
    if run.status == "cancelled" {
        svc.ctx
            .run_events
            .append(&params.run_id, "cancelled", None, now)?;
        // §6.2: wiped at run end — success, failure, or cancel.
        svc.creds.remove(&params.run_id);
    }
    Ok(run)
}
