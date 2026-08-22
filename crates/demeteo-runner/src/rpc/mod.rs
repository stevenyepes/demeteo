//! Control RPC server (docs/REMOTE_EXECUTION.md M3.1/M3.2/M4/M5).
//!
//! Listens on a Unix-domain socket at `<data_dir>/control.sock` with `0600`
//! permissions. Per the design doc's decided fork (REMOTE_EXECUTION.md
//! M3.1): protection comes from OS file permissions alone — no bearer
//! token, no listening TCP port. The laptop reaches this socket by
//! forwarding it over SSH (`ssh -L <local>:<remote.sock>`); a second local
//! user on the runner host cannot open a `0600` socket owned by a
//! different uid, so it is safe without additional authz.
//!
//! Wire format: newline-delimited JSON. One request per line in, one
//! response per line out — simple enough to test with `nc -U` or a raw
//! socket client, no RPC framework dependency.

mod credentials;
mod gates;
mod lifecycle;
mod ownership;
mod reads;

use crate::services::RunnerServices;
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
struct RunIdParams {
    run_id: String,
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
    // MC-D3: the owning client's `install_id` rides *inside* `params`
    // (not the `control_rpc` port signature), so it is extracted here,
    // generically, once, before method routing — every run-scoped handler
    // then funnels through `require_owner`. A caller that sends no
    // `client_id` (an old laptop) reads back as `""`, the single
    // legacy/unknown tenant, which is exactly how pre-V26 rows are stamped
    // — so old-client↔new-runner keeps working unchanged (MC-D6 / P0.6).
    let client_id = req
        .params
        .get("client_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cid = client_id.as_str();
    let result = match req.method.as_str() {
        "health" => Ok(serde_json::to_value(HealthInfo {
            version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
        })
        .unwrap()),
        "submit_run" => lifecycle::submit_run(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "probe_agent" => credentials::probe_agent(svc, req.params)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "inject_credentials" => credentials::inject_credentials(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "decide_gate" => gates::decide_gate(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "get_status" => reads::get_status(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "get_feature" => reads::get_feature(svc, req.params, cid)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "list_steps" => reads::list_steps(svc, req.params, cid)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "get_sequence_state" => reads::get_sequence_state(svc, req.params, cid)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "get_worktree" => reads::get_worktree(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "read_artifact" => reads::read_artifact(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "list_messages" => reads::list_messages(svc, req.params, cid)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "list_runs" => reads::list_runs(svc, cid)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "stream_events" => reads::stream_events(svc, req.params, cid)
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "cancel_run" => lifecycle::cancel_run(svc, req.params, cid)
            .await
            .and_then(|r| serde_json::to_value(r).map_err(|e| e.to_string())),
        "retry_step" => lifecycle::retry_step(svc, req.params, cid)
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
