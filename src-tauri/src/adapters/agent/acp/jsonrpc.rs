use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
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
    Request {
        id: Value,
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
            if let Some(id) = v.get("id").cloned() {
                return Ok(Message::Request { id, method, params });
            } else {
                return Ok(Message::Notification { method, params });
            }
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

/// Thin adapter that exposes our blocking `InteractiveHandle` as a
/// `std::io::Read`. The background reader hands this to serde_json's
/// `StreamDeserializer`, which transparently handles concatenated
/// messages, inter-value whitespace, and partial reads.
struct TransportAsRead(Arc<dyn InteractiveHandle>);

impl std::io::Read for TransportAsRead {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
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
    /// (one in-flight at a time). All methods take `&self` (interior
    /// mutability) so reads and writes don't share a single Mutex.
    transport: Arc<dyn InteractiveHandle>,
    next_id: AtomicU64,
    /// In-flight requests keyed by id.
    pending: Arc<Mutex<HashMap<u64, Pending>>>,
    /// Active notification / request sinks.
    notifs: Arc<Mutex<HashMap<u64, mpsc::UnboundedSender<Message>>>>,
}

impl Drop for JsonRpcClient {
    fn drop(&mut self) {
        // Kill the transport to unblock the background reader thread.
        let _ = self.transport.kill();
    }
}

impl JsonRpcClient {
    /// Forcibly tear down the transport. Closes the underlying SSH
    /// channel (which sends SIGHUP to the remote agent via the SSH
    /// server, killing the spawned `opencode acp`) or kills the local
    /// child. Idempotent and safe to call multiple times.
    ///
    /// Distinct from `Drop`: the background reader thread holds an
    /// `Arc` clone of the transport, so the transport isn't actually
    /// dropped (and thus `Drop::kill` doesn't run) until that thread
    /// exits. Explicit callers like the model probe need a way to
    /// close the channel now, not when the reader happens to notice
    /// EOF.
    pub fn kill(&self) -> Result<(), String> {
        self.transport.kill()
    }

    pub fn new(transport: Box<dyn InteractiveHandle>) -> Self {
        let transport: Arc<dyn InteractiveHandle> = Arc::from(transport);
        let next_id = AtomicU64::new(1);
        let pending = Arc::new(Mutex::new(HashMap::<u64, Pending>::new()));
        let notifs = Arc::new(Mutex::new(HashMap::<u64, mpsc::UnboundedSender<Message>>::new()));

        let t_clone = transport.clone();
        let p_clone = pending.clone();
        let n_clone = notifs.clone();

        std::thread::spawn(move || {
            let reader = TransportAsRead(t_clone);
            let mut stream =
                serde_json::Deserializer::from_reader(std::io::BufReader::new(reader))
                    .into_iter::<Message>();
            loop {
                let msg = match stream.next() {
                    Some(Ok(m)) => m,
                    Some(Err(e)) => {
                        eprintln!("[JsonRpcClient] background read error: {}", e);
                        let rpc_err = RpcError {
                            code: -32700,
                            message: format!("parse: {}", e),
                            data: None,
                        };
                        if let Ok(mut p) = p_clone.lock() {
                            for (_id, pending) in p.drain() {
                                let _ = pending.tx.send(Err(rpc_err.clone()));
                            }
                        }
                        break;
                    }
                    None => {
                        // EOF — agent closed stdout cleanly.
                        let rpc_err = RpcError {
                            code: -32001,
                            message: "agent closed stdout".into(),
                            data: None,
                        };
                        if let Ok(mut p) = p_clone.lock() {
                            for (_id, pending) in p.drain() {
                                let _ = pending.tx.send(Err(rpc_err.clone()));
                            }
                        }
                        break;
                    }
                };

                // Dispatch message
                match msg {
                    Message::Response { id, result, error } => {
                        let mut resolved = false;
                        if let Some(ref val) = id {
                            if let Some(n_u64) = val.as_u64() {
                                if let Ok(mut p) = p_clone.lock() {
                                    if let Some(pending) = p.remove(&n_u64) {
                                        let res = match error.clone() {
                                            Some(err) => Err(err),
                                            None => Ok(result.clone().unwrap_or(Value::Null)),
                                        };
                                        let _ = pending.tx.send(res);
                                        resolved = true;
                                    }
                                }
                            }
                        }
                        if !resolved {
                            if let Ok(n) = n_clone.lock() {
                                for tx in n.values() {
                                    let _ = tx.send(Message::Response {
                                        id: id.clone(),
                                        result: result.clone(),
                                        error: error.clone(),
                                    });
                                }
                            }
                        }
                    }
                    other => {
                        if let Ok(n) = n_clone.lock() {
                            for tx in n.values() {
                                let _ = tx.send(other.clone());
                            }
                        }
                    }
                }
            }
            eprintln!("[JsonRpcClient] background read thread stopped");
        });

        Self {
            transport,
            next_id,
            pending,
            notifs,
        }
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
            transport.write_line(&framed).map_err(|e| RpcError {
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

    /// Send a response back to the agent (e.g. for session/request_permission).
    pub async fn respond(
        &self,
        id: Value,
        result: Value,
    ) -> Result<(), RpcError> {
        let line = serde_json::to_string(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result,
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
            transport.write_line(&framed).map_err(|e| RpcError {
                code: -32603,
                message: format!("write: {}", e),
                data: None,
            })?;
            Ok(())
        })
        .await
        .map_err(|e| RpcError {
            code: -32000,
            message: format!("respond task: {}", e),
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
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let req = Request::new(id, method, params);

        let line = serde_json::to_string(&req).map_err(|e| RpcError {
            code: -32700,
            message: format!("serialize: {}", e),
            data: None,
        })?;
        let mut framed = line;
        framed.push('\n');

        let (tx, rx) = oneshot::channel::<Result<Value, RpcError>>();
        
        // Register pending oneshot
        {
            let mut p = self.pending.lock().map_err(|_| RpcError {
                code: -32000,
                message: "pending lock poisoned".into(),
                data: None,
            })?;
            p.insert(id, Pending { tx });
        }

        // Register notification channel
        let (notif_tx, mut notif_rx) = mpsc::unbounded_channel::<Message>();
        {
            let mut n = self.notifs.lock().map_err(|_| RpcError {
                code: -32000,
                message: "notifs lock poisoned".into(),
                data: None,
            })?;
            n.insert(id, notif_tx.clone());
        }

        // Drain notifications on a separate task; the user-supplied
        // sink lives in the caller's task, so we forward via mpsc.
        let drain = tokio::spawn(async move {
            let mut sink = notif_sink;
            while let Some(msg) = notif_rx.recv().await {
                sink(msg);
            }
        });

        // Write the request to transport.
        let transport = self.transport.clone();
        let write_res = tokio::task::spawn_blocking(move || {
            transport.write_line(&framed).map_err(|e| RpcError {
                code: -32603,
                message: format!("write: {}", e),
                data: None,
            })?;
            Ok::<(), RpcError>(())
        })
        .await;

        let _write_ok = match write_res {
            Ok(Ok(())) => true,
            Ok(Err(e)) => {
                // Deregister
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&id);
                }
                if let Ok(mut n) = self.notifs.lock() {
                    n.remove(&id);
                }
                return Err(e);
            }
            Err(e) => {
                if let Ok(mut p) = self.pending.lock() {
                    p.remove(&id);
                }
                if let Ok(mut n) = self.notifs.lock() {
                    n.remove(&id);
                }
                return Err(RpcError {
                    code: -32000,
                    message: format!("write task panicked: {}", e),
                    data: None,
                });
            }
        };

        // Await the response
        let res = rx.await;

        // Deregister notification channel
        {
            if let Ok(mut n) = self.notifs.lock() {
                n.remove(&id);
            }
        }

        // Drop the mpsc sender to finish the drain loop
        drop(notif_tx);
        let _ = drain.await;

        match res {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(e),
            Err(_) => Err(RpcError {
                code: -32000,
                message: "response oneshot dropped".into(),
                data: None,
            }),
        }
    }
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
