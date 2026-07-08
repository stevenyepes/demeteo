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
use demeteo_core::domain::ids::{FeatureId, ThreadId};
use demeteo_core::domain::models::{Feature, Message, StepExecution};
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
struct ReadArtifactParams {
    run_id: String,
    path: String,
}

#[derive(Debug, Deserialize)]
struct ListMessagesParams {
    run_id: String,
    thread_id: String,
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
        "get_feature" => get_feature(svc, req.params)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "list_steps" => list_steps(svc, req.params)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "read_artifact" => read_artifact(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "list_messages" => list_messages(svc, req.params)
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

/// Resolve a `run_id` to the `FeatureId` its background execution
/// bootstrapped. The C4 read-model RPCs (`get_feature`/`list_steps`/
/// `read_artifact`/`list_messages`) all key on `run_id` — the laptop's
/// idempotency key — and hop through the run's feature to reach the
/// engine's own `features`/`threads`/artifact state. `Err` if the run is
/// unknown or hasn't reached feature-bootstrap yet (nothing to render).
fn feature_id_for_run(svc: &Arc<RunnerServices>, run_id: &str) -> Result<FeatureId, String> {
    let run = svc
        .ctx
        .runner_runs
        .get(run_id)?
        .ok_or_else(|| format!("no such run: {}", run_id))?;
    let fid = run
        .feature_id
        .ok_or_else(|| format!("run {} has not bootstrapped a feature yet", run_id))?;
    Ok(FeatureId::from(fid))
}

/// C4.1: the runner's own `Feature` row for a run, so the laptop can
/// hydrate a read-only shadow of it (C4.2) and render it with the same
/// fidelity as a native feature (status/model/mr_url/aggregate cost).
fn get_feature(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<Option<Feature>, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id)?;
    svc.ctx.features.get(&fid)
}

/// C4.1: the run's step executions in creation order, each carrying its
/// own cost/tokens/artifact refs — the shadow step list the laptop
/// hydrates so `RunView::steps` serves a runner feature transparently.
fn list_steps(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<Vec<StepExecution>, String> {
    let params: RunIdParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id)?;
    svc.ctx.features.steps_for_feature(&fid)
}

/// C4.1: the UTF-8 body of one declared artifact, for the laptop's lazy
/// artifact cache (C4.2). **Guarded:** the requested `path` must be a
/// declared artifact of one of the run's steps — the control socket is
/// not a general remote-file read (a bare `read_file` would let any
/// tunnelled caller exfiltrate arbitrary files as the runner user). The
/// read itself goes through the engine's own `ExecutionPort`, which on
/// the runner is the local subprocess adapter (the runner *is* the
/// machine), and honours the port's error contract: a missing/unreadable
/// path is an `Err`, never `Ok("")`.
async fn read_artifact(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<String, String> {
    let params: ReadArtifactParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    let fid = feature_id_for_run(svc, &params.run_id)?;
    let steps = svc.ctx.features.steps_for_feature(&fid)?;
    let refs = steps
        .iter()
        .map(|s| (s.artifact_path.as_deref(), s.artifact_paths.as_slice()));
    if !is_declared_artifact(refs, &params.path) {
        return Err(format!(
            "path is not a declared artifact of run {}: {}",
            params.run_id, params.path
        ));
    }
    svc.ctx.exec.read_file("local", &params.path).await
}

/// The `read_artifact` guard: is `path` a declared artifact among these
/// steps' refs? A step declares artifacts via a single `artifact_path`
/// and/or a `artifact_paths` list; a match on either counts. Pure over
/// the two ref shapes (not `StepExecution`) so it's trivially testable
/// without building the full row.
fn is_declared_artifact<'a>(
    step_refs: impl IntoIterator<Item = (Option<&'a str>, &'a [String])>,
    path: &str,
) -> bool {
    step_refs
        .into_iter()
        .any(|(single, many)| single == Some(path) || many.iter().any(|p| p == path))
}

/// C4.1: a step's persisted agent transcript (the durable message
/// history `RunView::agent_stream` renders), so the laptop shadow can
/// show a runner run's conversation, not just its coarse event log.
///
/// `run_id` is accepted (and validated to exist + have a feature) so the
/// wire shape matches the other C4 read RPCs and a caller can't page a
/// thread on a run that never bootstrapped; the thread itself is trusted
/// by id, exactly as `decide_gate` trusts a bare `gate_id`. The socket's
/// `0600` + SSH-forwarding authz — the same boundary that already grants
/// the laptop full SFTP file read on this box — is the real access
/// control here, not a per-thread ownership check (the engine's
/// `thread_id` is a derived session key not stored on the step row, so
/// re-deriving it to gate reads would be brittle without adding any
/// boundary the tunnel doesn't already imply).
fn list_messages(
    svc: &Arc<RunnerServices>,
    params: serde_json::Value,
) -> Result<Vec<Message>, String> {
    let params: ListMessagesParams =
        serde_json::from_value(params).map_err(|e| format!("invalid params: {}", e))?;
    // Presence/bootstrap check only — surfaces a clear error for a bad
    // `run_id` instead of silently returning an empty transcript.
    feature_id_for_run(svc, &params.run_id)?;
    svc.ctx
        .threads
        .get_messages(&ThreadId::from(params.thread_id))
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

#[cfg(test)]
mod tests {
    use super::is_declared_artifact;

    fn refs<'a>(
        pairs: &'a [(Option<&'a str>, Vec<String>)],
    ) -> impl IntoIterator<Item = (Option<&'a str>, &'a [String])> {
        pairs.iter().map(|(s, m)| (*s, m.as_slice()))
    }

    #[test]
    fn matches_single_artifact_path() {
        let steps = [(Some("/w/report.md"), vec![])];
        assert!(is_declared_artifact(refs(&steps), "/w/report.md"));
    }

    #[test]
    fn matches_within_artifact_paths_list() {
        let steps = [(
            None,
            vec!["/w/a.txt".to_string(), "/w/b.txt".to_string()],
        )];
        assert!(is_declared_artifact(refs(&steps), "/w/b.txt"));
    }

    #[test]
    fn rejects_undeclared_path() {
        // The security-relevant case: a path no step declared must not
        // be readable over the control socket, even a plausible sibling.
        let steps = [
            (Some("/w/report.md"), vec!["/w/a.txt".to_string()]),
            (None, vec!["/w/b.txt".to_string()]),
        ];
        assert!(!is_declared_artifact(refs(&steps), "/w/../.ssh/id_rsa"));
        assert!(!is_declared_artifact(refs(&steps), "/w/report.md.bak"));
        assert!(!is_declared_artifact(refs(&steps), "/etc/passwd"));
    }

    #[test]
    fn rejects_when_no_steps_declare_anything() {
        let steps: [(Option<&str>, Vec<String>); 0] = [];
        assert!(!is_declared_artifact(refs(&steps), "/w/report.md"));
    }
}
