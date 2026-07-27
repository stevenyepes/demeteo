//! The control-socket RPC client for `demeteo-runner`: one request/response
//! round-trip over an SSH-forwarded Unix socket, plus the response decoding.
//! The `ExecutionPort` impl in `client.rs` keeps only the `spawn_blocking`
//! adaptation; the decode step is pure and lives here so the error-vs-result
//! contract is unit-testable without a runner or a socket.

use super::session::SessionPool;
use super::transport::{drain_stream, DrainBudget, TRANSPORT_WALL_CAP};
use std::io::Write;

#[derive(serde::Deserialize)]
struct RpcResponse {
    result: Option<serde_json::Value>,
    error: Option<String>,
}

/// Decode one line of the runner's control-socket response. Pure, so the
/// error-vs-result contract is testable without a runner or a socket.
fn parse_response(raw: &str) -> Result<serde_json::Value, String> {
    let line = raw
        .lines()
        .next()
        .ok_or_else(|| "empty response from demeteo-runner control socket".to_string())?;

    let resp: RpcResponse = serde_json::from_str(line)
        .map_err(|e| format!("invalid control-RPC response: {} (raw: {})", e, line))?;
    match resp.error {
        Some(e) => Err(e),
        None => Ok(resp.result.unwrap_or(serde_json::Value::Null)),
    }
}

/// M6.1: one request/response round-trip against `demeteo-runner`'s
/// control socket, reached via OpenSSH Unix-socket forwarding
/// (`channel_direct_streamlocal`, R4) over the same cached SSH session
/// `run_command`/SFTP use. Opens one fresh channel per call (the session
/// itself is what's cached/reused) — simple request/response, no
/// long-lived multiplexed connection to manage.
pub(super) fn call(
    pool: &SessionPool,
    machine_id: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let sftp_sess = pool.get(machine_id)?;
    let home = pool.resolve_home(machine_id)?;
    let socket_path = format!("{}/.local/share/demeteo-runner/control.sock", home);

    let mut channel = sftp_sess
        .session
        .channel_direct_streamlocal(&socket_path, None)
        .map_err(|e| {
            format!(
                "Failed to reach demeteo-runner control socket at {}: {} \
                 (is the runner installed and running on this machine?)",
                socket_path, e
            )
        })?;

    let request = serde_json::json!({ "id": 1u64, "method": method, "params": params });
    let mut line = serde_json::to_string(&request).map_err(|e| e.to_string())?;
    line.push('\n');
    channel
        .write_all(line.as_bytes())
        .map_err(|e| format!("Failed to write control-RPC request: {}", e))?;
    channel
        .flush()
        .map_err(|e| format!("Failed to flush control-RPC request: {}", e))?;
    // Half-close our write side so the runner's line-reader loop sees
    // EOF right after our one request and closes its side in turn —
    // that's what unblocks the `read_to_string` below.
    channel
        .send_eof()
        .map_err(|e| format!("Failed to send EOF on control-RPC channel: {}", e))?;

    let budget = DrainBudget::starting_now(TRANSPORT_WALL_CAP);
    let mut raw_bytes = Vec::new();
    drain_stream(
        &mut channel,
        &sftp_sess.session,
        &mut raw_bytes,
        budget,
        "control-RPC response",
    )?;
    let raw = String::from_utf8_lossy(&raw_bytes).into_owned();
    let _ = channel.close();
    let _ = channel.wait_close();

    parse_response(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No bytes at all is the shape of "the runner closed without answering" —
    /// a distinct failure from a malformed answer, so it gets its own message.
    #[test]
    fn empty_input_is_its_own_error() {
        let err = parse_response("").expect_err("empty must not decode");
        assert_eq!(err, "empty response from demeteo-runner control socket");
    }

    /// A non-JSON line quotes the raw text back, because the usual cause is the
    /// socket carrying something that isn't the runner (a shell banner, an
    /// error from the forwarder) and the raw bytes are the only clue.
    #[test]
    fn malformed_json_reports_the_raw_line() {
        let err = parse_response("not json at all\n").expect_err("garbage must not decode");
        assert!(
            err.starts_with("invalid control-RPC response: "),
            "unexpected error: {err}",
        );
        assert!(
            err.ends_with("(raw: not json at all)"),
            "the raw line must be quoted back: {err}",
        );
    }

    /// An `error` field wins over anything else and surfaces verbatim — the
    /// runner's message is the caller's message, not wrapped or re-prefixed.
    #[test]
    fn an_error_payload_surfaces_verbatim() {
        let err = parse_response(r#"{"id":1,"error":"no such run: abc"}"#)
            .expect_err("an error payload must be an Err");
        assert_eq!(err, "no such run: abc");
    }

    /// The happy path hands back the `result` value untouched.
    #[test]
    fn a_result_payload_is_returned_as_is() {
        let out = parse_response(r#"{"id":1,"result":{"status":"running","pid":42}}"#)
            .expect("a result payload must decode");
        assert_eq!(out, serde_json::json!({"status": "running", "pid": 42}));
    }

    /// Neither field set is an acknowledgement, not a failure: methods that
    /// return nothing decode to `Null` rather than erroring.
    #[test]
    fn neither_field_decodes_to_null() {
        let out = parse_response(r#"{"id":1}"#).expect("an empty ack must decode");
        assert_eq!(out, serde_json::Value::Null);
    }

    /// The protocol is one response per line and we only ever send one request,
    /// so anything after the first line is ignored rather than confusing the
    /// decoder.
    #[test]
    fn trailing_lines_after_the_first_are_ignored() {
        let out = parse_response("{\"id\":1,\"result\":1}\n{\"id\":2,\"error\":\"later noise\"}\n")
            .expect("the first line must decide the outcome");
        assert_eq!(out, serde_json::json!(1));
    }
}
