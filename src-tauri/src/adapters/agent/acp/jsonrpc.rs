use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;
use tokio::sync::oneshot;

use crate::ports::execution::InteractiveHandle;

/// A JSON-RPC 2.0 request we send *to* the agent.
#[derive(Debug, Clone, Serialize)]
pub struct Request {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    pub params: Value,
}

impl Request {
    pub fn new(id: u64, method: impl Into<String>, params: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// A JSON-RPC 2.0 message coming *from* the agent.
///
/// Discriminated manually because `untagged` can't tell a notification
/// (`{method, params}`) apart from a response with all-`None` fields —
/// both are valid JSON. We try `Notification` first (matching on the
/// `method` key), then fall back to `Response`.
#[derive(Debug, Clone)]
pub enum Message {
    Response {
        id: Option<Value>,
        result: Option<Value>,
        error: Option<RpcError>,
    },
    Notification {
        method: String,
        params: Option<Value>,
    },
}

impl<'de> Deserialize<'de> for Message {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        let v = serde_json::Value::deserialize(deserializer)?;
        if v.get("method").is_some() {
            let method = v
                .get("method")
                .and_then(|m| m.as_str())
                .ok_or_else(|| D::Error::custom("notification missing method string"))?
                .to_string();
            let params = v.get("params").cloned();
            return Ok(Message::Notification { method, params });
        }
        let id = v.get("id").cloned();
        let result = v.get("result").cloned();
        let error = v
            .get("error")
            .map(|e| serde_json::from_value::<RpcError>(e.clone()))
            .transpose()
            .map_err(D::Error::custom)?;
        Ok(Message::Response { id, result, error })
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "rpc error {}: {}", self.code, self.message)
    }
}

impl std::error::Error for RpcError {}

struct Pending {
    tx: oneshot::Sender<Result<Value, RpcError>>,
}

/// Synchronous JSON-RPC 2.0 line client. Reads newline-delimited JSON
/// messages from the agent's stdout and routes responses to the matching
/// pending request; notifications get pushed to a single sink. The
/// `prompt` loop in `AcpRuntime` drains the notification sink while it
/// awaits the prompt response.
///
/// Wire format: one JSON object per line, with a trailing `\n` delimiter.
pub struct JsonRpcClient {
    /// The underlying agent process. The AcpRuntime serializes calls
    /// (one in-flight at a time).
    transport: Arc<Mutex<Box<dyn InteractiveHandle>>>,
    next_id: AtomicU64,
    /// In-flight requests keyed by id.
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
}

impl JsonRpcClient {
    pub fn new(transport: Box<dyn InteractiveHandle>) -> Self {
        Self {
            transport: Arc::new(Mutex::new(transport)),
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Borrow the transport for the AcpRuntime's lifecycle (kill, etc).
    pub fn transport(&self) -> Arc<Mutex<Box<dyn InteractiveHandle>>> {
        self.transport.clone()
    }

    /// Send a notification (no `id`, no response expected). Used for
    /// `tool_call/update` from the client back to the agent — the agent
    /// is mid-prompt and won't reply to this message; we just write it
    /// and return.
    pub async fn notify(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(), RpcError> {
        // JSON-RPC 2.0 notification: no `id` field.
        let line = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .map_err(|e| RpcError {
            code: -32700,
            message: format!("serialize: {}", e),
            data: None,
        })?;
        let mut framed = line;
        framed.push('\n');
        let transport = self.transport.clone();
        tokio::task::spawn_blocking(move || -> Result<(), RpcError> {
            let mut t = transport.lock().map_err(|_| RpcError {
                code: -32000,
                message: "transport lock poisoned".into(),
                data: None,
            })?;
            t.write_line(&framed).map_err(|e| RpcError {
                code: -32603,
                message: format!("write: {}", e),
                data: None,
            })?;
            Ok(())
        })
        .await
        .map_err(|e| RpcError {
            code: -32000,
            message: format!("notify task: {}", e),
            data: None,
        })?
    }

    /// Send a request and block on the response. While blocked, the read
    /// loop drains incoming notifications to `notif_sink` so they don't
    /// queue up behind our response.
    ///
    /// Async because the underlying transport is sync (blocking I/O); we
    /// hop to a blocking pool so we don't starve the tokio worker. The
    /// call returns when the matching response arrives or the transport
    /// dies.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
        notif_sink: impl FnMut(Message) + Send + 'static,
    ) -> Result<Value, RpcError> {
        let transport = self.transport.clone();
        let pending = self.pending.clone();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel::<Message>();
        let (result_tx, result_rx) = oneshot::channel::<Result<Value, RpcError>>();

        // Drain notifications on a separate task; the user-supplied
        // sink lives in the caller's task, so we forward via mpsc.
        let drain = tokio::spawn(async move {
            let mut sink = notif_sink;
            while let Some(msg) = notif_rx.recv().await {
                sink(msg);
            }
        });

        let req = Request::new(id, method, params);
        let _ = tokio::task::spawn_blocking(move || {
            let res = call_blocking(&transport, &pending, id, &req, notif_tx);
            let _ = result_tx.send(res);
        })
        .await;

        let _ = drain.await;
        result_rx.await.map_err(|_| RpcError {
            code: -32000,
            message: "jsonrpc call task dropped".into(),
            data: None,
        })?
    }
}

/// The blocking half of `JsonRpcClient::call`. Runs on a
/// `spawn_blocking` worker so it doesn't pin a tokio reactor thread.
fn call_blocking(
    transport: &Arc<Mutex<Box<dyn InteractiveHandle>>>,
    pending: &Arc<Mutex<HashMap<u64, Pending>>>,
    id: u64,
    req: &Request,
    notif_tx: mpsc::UnboundedSender<Message>,
) -> Result<Value, RpcError> {
    let line = serde_json::to_string(req).map_err(|e| RpcError {
        code: -32700,
        message: format!("serialize: {}", e),
        data: None,
    })?;
    let mut framed = line;
    framed.push('\n');

    // Register the pending response channel.
    let (tx, rx) = oneshot::channel::<Result<Value, RpcError>>();
    {
        let mut p = pending.lock().map_err(|_| RpcError {
            code: -32000,
            message: "pending lock poisoned".into(),
            data: None,
        })?;
        p.insert(id, Pending { tx });
    }

    // Send the request.
    {
        let mut transport = transport.lock().map_err(|_| RpcError {
            code: -32000,
            message: "transport lock poisoned".into(),
            data: None,
        })?;
        transport.write_line(&framed).map_err(|e| RpcError {
            code: -32603,
            message: format!("write: {}", e),
            data: None,
        })?;
    }

    // Read loop: pull messages until our `id` arrives or the transport
    // dies. Notifications get pushed to the mpsc; only the response
    // with the matching `id` resolves the oneshot.
    loop {
        let msg = read_one_message_blocking(transport)?;
        match msg {
            Message::Response {
                id: Some(Value::Number(n)),
                result,
                error,
            } => {
                let n = n.as_u64().unwrap_or(0);
                if n == id {
                    let mut p = pending.lock().map_err(|_| RpcError {
                        code: -32000,
                        message: "pending lock poisoned".into(),
                        data: None,
                    })?;
                    if let Some(pending) = p.remove(&id) {
                        let res = match error {
                            Some(e) => Err(e),
                            None => Ok(result.unwrap_or(Value::Null)),
                        };
                        let _ = pending.tx.send(res);
                    }
                    return match rx.blocking_recv() {
                        Ok(Ok(v)) => Ok(v),
                        Ok(Err(e)) => Err(e),
                        Err(_) => Err(RpcError {
                            code: -32000,
                            message: "response oneshot dropped".into(),
                            data: None,
                        }),
                    };
                }
                // Stale id; drop on the floor.
            }
            Message::Response { id, result, error } => {
                let _ = notif_tx.send(Message::Response { id, result, error });
            }
            Message::Notification { method, params } => {
                let _ = notif_tx.send(Message::Notification { method, params });
            }
        }
    }
}

fn read_one_message_blocking(
    transport: &Arc<Mutex<Box<dyn InteractiveHandle>>>,
) -> Result<Message, RpcError> {
    let mut line = String::new();
    loop {
        let read_result = {
            let mut t = transport.lock().map_err(|_| RpcError {
                code: -32000,
                message: "transport lock poisoned".into(),
                data: None,
            })?;
            t.read_byte()
        };
        match read_result {
            Ok(b) => {
                if b == b'\n' {
                    break;
                }
                line.push(b as char);
            }
            Err(e) if e.kind() == io::ErrorKind::TimedOut => {
                std::thread::sleep(std::time::Duration::from_millis(20));
                continue;
            }
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                return Err(RpcError {
                    code: -32001,
                    message: "agent closed stdout".into(),
                    data: None,
                });
            }
            Err(e) => {
                return Err(RpcError {
                    code: -32603,
                    message: format!("read: {}", e),
                    data: None,
                });
            }
        }
    }

    eprintln!("[agent jsonrpc] {}", line);

    let msg: Message = serde_json::from_str(&line).map_err(|e| RpcError {
        code: -32700,
        message: format!("parse: {} (line: {})", e, line),
        data: None,
    })?;
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_serializes_with_jsonrpc_envelope() {
        let req = Request::new(7, "session/prompt", serde_json::json!({"text": "hi"}));
        let s = serde_json::to_string(&req).unwrap();
        assert!(s.contains("\"jsonrpc\":\"2.0\""));
        assert!(s.contains("\"id\":7"));
        assert!(s.contains("\"method\":\"session/prompt\""));
    }

    #[test]
    fn notification_parses_without_id() {
        let m: Message = serde_json::from_str(
            "{\"method\":\"session/update\",\"params\":{\"kind\":\"text\",\"delta\":\"hi\"}}",
        )
        .unwrap();
        match m {
            Message::Notification { method, params } => {
                assert_eq!(method, "session/update");
                assert!(params.is_some());
            }
            _ => panic!("expected Notification"),
        }
    }

    #[test]
    fn response_with_error_parses() {
        let m: Message = serde_json::from_str(
            "{\"id\":3,\"error\":{\"code\":-32601,\"message\":\"method not found\"}}",
        )
        .unwrap();
        match m {
            Message::Response { error, .. } => {
                assert_eq!(error.unwrap().code, -32601);
            }
            _ => panic!("expected Response"),
        }
    }

    #[test]
    fn response_with_result_parses() {
        let m: Message =
            serde_json::from_str("{\"id\":1,\"result\":{\"sessionId\":\"s1\"}}").unwrap();
        match m {
            Message::Response { id, result, error } => {
                assert!(id.is_some());
                assert!(error.is_none());
                assert_eq!(result.unwrap()["sessionId"], "s1");
            }
            _ => panic!("expected Response"),
        }
    }
}
