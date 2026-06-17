use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::json;
use tokio::sync::mpsc;
use tokio_stream::Stream;

use tokio::sync::oneshot;

use crate::adapters::agent::acp::event_mapper::{
    map_session_update, map_tool_call_update, map_usage_notification,
};
use crate::adapters::agent::acp::jsonrpc::{JsonRpcClient, Message, RpcError};
use crate::adapters::agent::acp::tool_bridge::{DispatchResult, ToolBridge};
use crate::domain::agent_event::AgentEvent;
use crate::domain::intercept::ExecutionResult;
use crate::domain::models::SessionInfo;
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};

/// Concrete runtime for agents speaking [ACP](https://agentclientprotocol.com/).
/// Owns the JSON-RPC client, runs `initialize` + `session/new` on `start`,
/// and produces a per-turn `AgentEvent` stream when `prompt` is called.
///
/// The runtime is generic over the `AgentContext`; concrete adapters
/// (`opencode`, `hermes`) live in their own modules and only differ in
/// the binary name, args, and install command.
pub struct AcpRuntime {
    pub kind: &'static str,
    pub install_command: &'static str,
}

impl AcpRuntime {
    pub const fn new(kind: &'static str, install_command: &'static str) -> Self {
        Self { kind, install_command }
    }
}

impl AgentRuntime for AcpRuntime {
    fn kind(&self) -> &'static str {
        self.kind
    }

    fn is_available(&self, exec: &dyn crate::ports::execution::ExecutionPort, machine_id: &str) -> bool {
        if machine_id == "local" || machine_id.is_empty() {
            is_binary_on_path(self.kind, machine_id)
        } else {
            exec.run_command(machine_id, &format!("command -v {} >/dev/null 2>&1 && echo ok", self.kind))
                .map(|out| out.trim() == "ok")
                .unwrap_or(false)
        }
    }

    fn install_command(&self) -> &'static str {
        self.install_command
    }

    fn start(
        &self,
        ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>>
    {
        Box::pin(async move {
            // Spawn the agent process over the right transport. We resolve
            // "local" via the OS PATH lookup; for remote machines, the
            // SshClientAdapter's spawn_interactive handles the channel.
            let transport = spawn_transport(&ctx)
                .map_err(AgentStartError::SpawnFailed)?;

            let rpc = Arc::new(JsonRpcClient::new(transport));

            // Capability negotiation.
            let init_params = json!({
                "protocolVersion": 1,
                "clientInfo": {
                    "name": "demeteo",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {
                    "fs": { "readTextFile": true, "writeTextFile": true },
                    "terminal": true
                }
            });
            let notif_collector: Arc<StdMutex<Vec<Message>>> = Arc::new(StdMutex::new(Vec::new()));
            let nc = notif_collector.clone();
            let init_result = rpc
                .call("initialize", init_params, move |m| nc.lock().unwrap().push(m))
                .await
                .map_err(|e| AgentStartError::SpawnFailed(format!("initialize: {}", e)))?;
            // Notifications from `initialize` aren't currently used;
            // the agent's init response carries the capability surface.
            drop(notif_collector);

            // Detect whether the agent supports receiving `tool_call/update`
            // notifications from us. Opencode (and other ACP v1 agents) may
            // not implement that inbound method; sending it causes a noisy
            // -32601 on the agent's stderr. We check two naming conventions
            // (camelCase and snake_case) to be forward-compatible.
            let (supports_tool_call_update, tool_call_update_method) = init_result
                .get("capabilities")
                .map(|c| {
                    if c.get("toolCallUpdate").and_then(|v| v.as_bool()).unwrap_or(false) {
                        (true, "toolCall/update".to_string())
                    } else if c.get("tool_call_update").and_then(|v| v.as_bool()).unwrap_or(false) {
                        (true, "tool_call/update".to_string())
                    } else {
                        (false, "tool_call/update".to_string())
                    }
                })
                .unwrap_or((false, "tool_call/update".to_string()));

            // If the agent advertises authMethods, call authenticate. v1
            // doesn't surface agent API keys (see spec §1) so we just pass
            // through with a no-op; the agent is pre-configured on the host.
            // The schema requires `methodId` (a string) — pass the first
            // method's `id` so the agent's Zod validator accepts the call.
            if let Some(method_id) = init_result
                .get("authMethods")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|m| m.get("id"))
                .and_then(|v| v.as_str())
            {
                let _ = rpc
                    .call("authenticate", json!({ "methodId": method_id }), |_| {})
                    .await;
            }

            // Create the per-turn session. The schema requires
            // `mcpServers` (an array, may be empty) alongside `cwd`.
            let session_result = rpc
                .call(
                    "session/new",
                    json!({ "cwd": ctx.cwd, "mcpServers": [] }),
                    |_| {},
                )
                .await
                .map_err(|e| AgentStartError::SpawnFailed(format!("session/new: {}", e)))?;
            let session_id = session_result
                .get("sessionId")
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    AgentStartError::SpawnFailed("session/new: missing sessionId".into())
                })?
                .to_string();

            // Capture session info (modes, config options, etc.) from
            // the session/new response so the frontend can display
            // available modes/models for interactive selection.
            let session_info = SessionInfo {
                modes: session_result
                    .get("modes")
                    .map(|v| serde_json::from_value(v.clone()).ok())
                    .flatten(),
                config_options: session_result
                    .get("configOptions")
                    .map(|v| serde_json::from_value(v.clone()).ok())
                    .flatten(),
                raw: Some(
                    serde_json::from_value(session_result.clone())
                        .unwrap_or_default(),
                ),
            };

            // Tool bridge: we build it here so `prompt` can dispatch
            // agent-originated file/terminal requests through the
            // existing policy + scope-fence machinery.
            let bridge = Arc::new(crate::adapters::agent::acp::tool_bridge::ToolBridge::new(
                ctx.agent_exec.clone(),
            ));
            Ok(Arc::new(AcpSession {
                rpc,
                session_id,
                kind: self.kind.to_string(),
                ctx,
                cancelled: AtomicBool::new(false),
                prompting: Arc::new(AtomicBool::new(false)),
                bridge,
                session_info: StdMutex::new(session_info),
                supports_tool_call_update,
                tool_call_update_method,
            }) as Arc<dyn AgentSession>)
        })
    }
}

fn spawn_transport(ctx: &AgentContext) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
    if ctx.machine_id == "local" || ctx.machine_id.is_empty() {
        // Local case: spawn via tokio::process::Command.
        spawn_local(ctx)
    } else {
        // Remote case: spawn via the SSH client adapter.
        ctx.exec.spawn_interactive(&ctx.machine_id, &ctx.binary, &ctx.args, &ctx.cwd, &ctx.env)
    }
}

fn spawn_local(ctx: &AgentContext) -> Result<Box<dyn crate::ports::execution::InteractiveHandle>, String> {
    use crate::adapters::agent::acp::transport_local::LocalSubprocessTransport;
    let t = LocalSubprocessTransport::spawn(&ctx.binary, &ctx.args, &ctx.cwd, &ctx.env)?;
    Ok(Box::new(t))
}

/// Probe the system for the agent binary. For v1 we only check the local
/// PATH; remote availability is verified by the SSH adapter in a future
/// phase. The "machine_id == local" sentinel bypasses the check.
fn is_binary_on_path(binary: &str, machine_id: &str) -> bool {
    if machine_id == "local" || machine_id.is_empty() {
        // Use `command -v` via std::process to avoid adding a `which`
        // dependency. Returns 0 if the binary is on PATH.
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {} >/dev/null 2>&1", binary))
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    } else {
        // Remote check is the registry's job (via SSH `command -v`).
        // We return `true` optimistically here and let the spawn fail
        // with a clear error if the binary isn't there.
        true
    }
}

struct AcpSession {
    rpc: Arc<JsonRpcClient>,
    session_id: String,
    kind: String,
    ctx: AgentContext,
    cancelled: AtomicBool,
    /// True while `run_prompt_inner` has an in-flight `session/prompt`
    /// RPC. Prevents `set_config_option` from opening a second
    /// `call_blocking` reader on the same transport (which would
    /// corrupt both message streams).
    prompting: Arc<AtomicBool>,
    bridge: Arc<ToolBridge>,
    session_info: StdMutex<SessionInfo>,
    /// Whether the agent declared `capabilities.toolCallUpdate` in its
    /// `initialize` response. When false, we skip sending `tool_call/update`
    /// notifications to avoid -32601 errors on the agent's stderr.
    supports_tool_call_update: bool,
    /// The resolved method name for tool call updates (e.g. `toolCall/update` or `tool_call/update`).
    tool_call_update_method: String,
}

impl AcpSession {
    fn run_prompt_inner(&self, text: String) -> mpsc::Receiver<AgentEvent> {
        let (tx, rx) = mpsc::channel::<AgentEvent>(64);
        let rpc = self.rpc.clone();
        let session_id = self.session_id.clone();
        let bridge = self.bridge.clone();
        let machine_id = self.ctx.machine_id.clone();
        let thread_id = self.ctx.thread_id.clone();
        let prompting = self.prompting.clone();
        let supports_tool_call_update = self.supports_tool_call_update;
        let tool_call_update_method = self.tool_call_update_method.clone();
        prompting.store(true, Ordering::SeqCst);
        tokio::spawn(async move {
            // `session/prompt` is a long-running call: the agent streams
            // notifications until it sends a response. Our JSON-RPC
            // client's `call` drains notifications into a sink. We spawn
            // tokio tasks to process each notification in real-time
            // (streaming text to the UI and executing tool calls via
            // the bridge), keeping track of the tasks so we can await
            // them before finishing the turn.
            let tx_clone = tx.clone();
            let rpc_clone = rpc.clone();
            let bridge_clone = bridge.clone();
            let machine_id_clone = machine_id.clone();
            let thread_id_clone = thread_id.clone();
            let tool_call_update_method_clone = tool_call_update_method.clone();

            let pending_tasks = Arc::new(StdMutex::new(Vec::new()));
            let pending_tasks_clone = pending_tasks.clone();

            let result = rpc
                .call(
                    "session/prompt",
                    json!({
                        "sessionId": session_id,
                        "prompt": [{
                            "type": "text",
                            "text": text
                        }]
                    }),
                    move |msg| {
                        let tx = tx_clone.clone();
                        let rpc = rpc_clone.clone();
                        let bridge = bridge_clone.clone();
                        let machine_id = machine_id_clone.clone();
                        let thread_id = thread_id_clone.clone();
                        let tc_method = tool_call_update_method_clone.clone();
                        let handle = tokio::spawn(async move {
                            handle_message(&tx, &rpc, &bridge, &machine_id, &thread_id, supports_tool_call_update, &tc_method, msg).await;
                        });
                        pending_tasks_clone.lock().unwrap().push(handle);
                    },
                )
                .await;

            // Wait for all pending notification handling tasks to finish
            // so we don't send TurnComplete prematurely.
            let handles: Vec<_> = std::mem::take(&mut *pending_tasks.lock().unwrap());
            for h in handles {
                let _ = h.await;
            }

            match result {
                Ok(value) => {
                    let stop_reason = value
                        .get("stopReason")
                        .or_else(|| value.get("stop_reason"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("end_of_turn");
                    let _ = tx
                        .send(AgentEvent::TurnComplete {
                            stop_reason: stop_reason_from_str(stop_reason),
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(AgentEvent::Error {
                            code: error_code(&e),
                            message: e.message.clone(),
                            recoverable: e.code != -32001,
                        })
                        .await;
                    let _ = tx
                        .send(AgentEvent::TurnComplete {
                            stop_reason: stop_reason_from_error(&e),
                        })
                        .await;
                }
            }
            prompting.store(false, Ordering::SeqCst);
        });
        rx
    }
}

/// Dispatch a single buffered message: emit it as an `AgentEvent` to
/// the consumer, and if it's a `ToolCall` notification, also drive the
/// tool bridge so the agent sees the policy result inline.
///
/// `ToolBridge` is a per-thread gate. Per spec §5.4, the bridge routes
/// the request through `PolicyEnforcedExecutionPort::submit_agent`
/// with the `tool_call_id` recorded; a `Policy::Reject` becomes a
/// `tool_call/update { status: Failed, content: [...] }` JSON-RPC
/// response sent back to the agent.
async fn handle_message(
    tx: &mpsc::Sender<AgentEvent>,
    rpc: &Arc<JsonRpcClient>,
    bridge: &Arc<ToolBridge>,
    machine_id: &str,
    thread_id: &str,
    supports_tool_call_update: bool,
    tool_call_update_method: &str,
    msg: Message,
) {
    if let Message::Request { id, method, params } = &msg {
        if method == "session/request_permission" {
            // Find the allow option or default to "once"
            let option_id = params
                .as_ref()
                .and_then(|p| p.get("options"))
                .and_then(|o| o.as_array())
                .and_then(|arr| {
                    // Try to find allow_once first (for stateless agent handling), then allow_always
                    arr.iter()
                        .find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_once"))
                        .or_else(|| arr.iter().find(|opt| opt.get("kind").and_then(|k| k.as_str()) == Some("allow_always")))
                        .or_else(|| arr.first())
                        .and_then(|opt| opt.get("optionId").cloned())
                })
                .unwrap_or(serde_json::json!("once"));

            eprintln!("[AcpRuntime] handle_message: auto-approving request_permission with optionId={:?}", option_id);

            let res = rpc.respond(id.clone(), serde_json::json!({
                "outcome": {
                    "outcome": "selected",
                    "optionId": option_id
                }
            })).await;

            if let Err(e) = res {
                eprintln!("[AcpRuntime] handle_message: failed to send request_permission response: {}", e);
            }
        }
    }

    if let Message::Notification { method, params } = &msg {
        if method == "session/update" {
            let p = params.clone().unwrap_or(json!({}));
            let mut is_tool_call = false;
            let mut tool_call_params = p.clone();

            if p.get("kind").and_then(|v| v.as_str()) == Some("tool_call") {
                is_tool_call = true;
            } else if let Some(update) = p.get("update") {
                if update.get("sessionUpdate").and_then(|v| v.as_str()) == Some("tool_call") {
                    is_tool_call = true;
                    let tool_call_id = update
                        .get("toolCallId")
                        .or_else(|| update.get("id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let raw = update.get("rawInput").or_else(|| update.get("input"));
                    let action_str = raw
                        .and_then(|r| r.get("action"))
                        .and_then(|v| v.as_str())
                        .or_else(|| update.get("kind").and_then(|v| v.as_str()))
                        .unwrap_or("read");
                    let target = raw
                        .and_then(|r| r.get("path").or_else(|| r.get("cmd")))
                        .and_then(|v| v.as_str())
                        .or_else(|| update.get("title").and_then(|v| v.as_str()))
                        .unwrap_or("");
                    let content = raw
                        .and_then(|r| r.get("content"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");

                    tool_call_params = json!({
                        "toolCallId": tool_call_id,
                        "action": action_str,
                        "target": target,
                        "content": content
                    });
                }
            }

            if is_tool_call && supports_tool_call_update {
                if let Some(dispatch) = build_tool_call_response(bridge, machine_id, thread_id, &tool_call_params) {
                    if let Some(payload) = dispatch.immediate {
                        let _ = rpc.notify(tool_call_update_method, payload).await;
                    }
                    if let Some(rx) = dispatch.intercept_rx {
                        let rpc = rpc.clone();
                        let tc_id = dispatch.tool_call_id;
                        let tc_method = tool_call_update_method.to_string();
                        tokio::spawn(async move {
                            match rx.await {
                                Ok(Ok(r)) => {
                                    let body = result_to_json(&r);
                                    let _ = rpc.notify(&tc_method, json!({
                                        "toolCallId": tc_id,
                                        "status": "completed",
                                        "content": [{ "type": "text", "text": serde_json::to_string(&body).unwrap_or_default() }],
                                    })).await;
                                }
                                Ok(Err(e)) => {
                                    let _ = rpc.notify(&tc_method, json!({
                                        "toolCallId": tc_id,
                                        "status": "failed",
                                        "content": [{ "type": "text", "text": e }],
                                    })).await;
                                }
                                Err(_) => {
                                    let _ = rpc.notify(&tc_method, json!({
                                        "toolCallId": tc_id,
                                        "status": "failed",
                                        "content": [{ "type": "text", "text": "intercept channel closed" }],
                                    })).await;
                                }
                            }
                        });
                    }
                }
            }
        }
    }
    emit_message(tx, msg);
}

struct ToolCallDispatch {
    /// Payload to send as `tool_call/update` immediately (executed/rejected).
    immediate: Option<serde_json::Value>,
    /// Intercept result receiver — present when action was intercepted.
    intercept_rx: Option<oneshot::Receiver<Result<ExecutionResult, String>>>,
    /// The tool_call_id from the agent, needed to correlate the response.
    tool_call_id: String,
}

/// Build the response for a tool call by routing through the bridge.
/// Returns immediate payload for executed/rejected actions, or a receiver
/// that will get the result once the supervisor approves/rejects.
fn build_tool_call_response(
    bridge: &Arc<ToolBridge>,
    machine_id: &str,
    thread_id: &str,
    params: &serde_json::Value,
) -> Option<ToolCallDispatch> {
    use crate::domain::action::ActionKind;
    let tool_call_id = params.get("toolCallId").and_then(|v| v.as_str())?;
    let action_str = params.get("action").and_then(|v| v.as_str())?;
    let target = params.get("target").and_then(|v| v.as_str()).unwrap_or("");
    let kind = match action_str {
        "read" | "read_text_file" | "fs/read_text_file" => ActionKind::Read,
        "write" | "write_text_file" | "fs/write_text_file" => ActionKind::Write,
        "edit" | "edit_text_file" | "fs/edit_text_file" => ActionKind::Edit,
        "run_bash" | "bash" | "terminal/create" => ActionKind::RunBash,
        _ => return None,
    };

    let dr: DispatchResult = match kind {
        ActionKind::Read => {
            bridge.handle_read_text_file(thread_id, machine_id, target, tool_call_id)
        }
        ActionKind::Write => {
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            bridge.handle_write_text_file(thread_id, machine_id, target, content, tool_call_id)
        }
        ActionKind::Edit => {
            let content = params
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            bridge.handle_edit_text_file(thread_id, machine_id, target, content, tool_call_id)
        }
        ActionKind::RunBash => {
            bridge.handle_terminal_create(thread_id, machine_id, target, tool_call_id)
        }
    };

    let immediate = dr.payload.map(|body| {
        serde_json::json!({
            "toolCallId": tool_call_id,
            "status": "completed",
            "content": [{ "type": "text", "text": serde_json::to_string(&body).unwrap_or_default() }],
        })
    });

    Some(ToolCallDispatch {
        immediate,
        intercept_rx: dr.intercept_rx,
        tool_call_id: tool_call_id.to_string(),
    })
}

fn result_to_json(r: &ExecutionResult) -> serde_json::Value {
    match r {
        ExecutionResult::Bash { output } => json!({ "output": output }),
        ExecutionResult::FileChanged { path, lines_added, lines_removed } => {
            json!({ "path": path, "lines_added": lines_added, "lines_removed": lines_removed })
        }
        ExecutionResult::FileRead { path: _, content_preview } => {
            json!({ "content": content_preview, "truncated": true })
        }
    }
}

impl AgentSession for AcpSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        // Reset cancel flag for the new turn.
        self.cancelled.store(false, Ordering::SeqCst);

        // Serialize prompts: only one session/prompt is in flight per
        // session at a time. The new prompt waits for the previous
        // one to finish via the JsonRpcClient's transport mutex. The
        // frontend's `sendDirective` issues a `session/cancel` first
        // when the prior turn is still running, so the previous
        // `call_blocking` returns promptly with the cancel response.
        let rx = self.run_prompt_inner(text.to_string());
        Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
    }

    fn cancel(&self) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        // Best-effort: send a `session/cancel` notification. The agent
        // will stop the in-flight turn; we'll get a TurnComplete with
        // stop_reason=cancelled in the next prompt. The cancel is
        // fire-and-forget: we spawn it and return immediately so the
        // sync trait method doesn't have to block on async I/O.
        let rpc = self.rpc.clone();
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            let _ = rpc
                .call("session/cancel", json!({ "sessionId": session_id }), |_| {})
                .await;
        });
        Ok(())
    }

    fn set_mode(&self, mode_id: &str) -> Result<(), String> {
        if self.prompting.load(Ordering::SeqCst) {
            return Ok(());
        }
        let rpc = self.rpc.clone();
        let session_id = self.session_id.clone();
        let mode = mode_id.to_string();

        let result = tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    rpc.call("session/set_mode", json!({ "sessionId": session_id, "modeId": mode }), |_| {})
                ).await
            })
        });

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("set_mode failed: {} ({})", e.message, e.code)),
            Err(_) => {
                eprintln!("[acp/runtime] set_mode timed out after 10s!");
                Ok(())
            }
        }
    }

    fn set_config_option(&self, config_id: &str, value: &str) -> Result<(), String> {
        // Update local cache immediately so session_info() reflects the
        // change right away (the selector UI reads from this).
        if let Ok(mut info) = self.session_info.lock() {
            if let Some(ref mut opts) = info.config_options {
                if let Some(opt) = opts.iter_mut().find(|o| o.id == config_id) {
                    opt.current_value = value.to_string();
                }
            }
        }

        // If a prompt is in-flight, skip the RPC call entirely.
        // Two concurrent call_blocking readers on the same JSON-RPC
        // transport corrupt each other's message buffers. The DB is
        // already persisted by the caller, and apply_thread_model
        // will re-apply the model from DB on the next prompt.
        if self.prompting.load(Ordering::SeqCst) {
            return Ok(());
        }

        // No prompt in-flight — safe to do a blocking RPC call.
        // Use a timeout so we don't block forever if the agent is
        // unresponsive (e.g. crashed, as indicated by
        // NeedDebuggerBreak traps).
        let rpc = self.rpc.clone();
        let session_id = self.session_id.clone();
        let cid = config_id.to_string();
        let val = value.to_string();

        let result = tokio::task::block_in_place(|| {
            let handle = tokio::runtime::Handle::current();
            handle.block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    rpc.call("session/set_config_option", json!({ "sessionId": session_id, "configId": cid, "value": val }), |_| {})
                ).await
            })
        });

        match result {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(format!("set_config_option failed: {} ({})", e.message, e.code)),
            Err(_) => {
                // Timeout — treat as success since DB is already
                // persisted and apply_thread_model will retry on
                // the next prompt.
                eprintln!("[acp/runtime] set_config_option timed out after 10s!");
                Ok(())
            }
        }
    }

    fn session_info(&self) -> SessionInfo {
        self.session_info.lock().unwrap().clone()
    }
}

fn emit_message(tx: &mpsc::Sender<AgentEvent>, msg: Message) {
    match msg {
        Message::Notification { method, params } => {
            let params = params.unwrap_or(json!({}));
            if method == "session/update" {
                for ev in map_session_update(&params) {
                    // The sender side is on a `spawn_blocking` worker;
                    // we use `try_send` (sync) so we don't block a tokio
                    // thread. The channel is bounded (64); if the
                    // consumer falls behind, we drop the event and the
                    // watchdog will surface a hang.
                    let _ = tx.try_send(ev);
                }
            } else if method == "tool_call/update" || method == "toolCall/update" {
                if let Some(ev) = map_tool_call_update(&params) {
                    let _ = tx.try_send(ev);
                }
            } else if method == "session/usage_update" {
                if let Some(ev) = map_usage_notification(&params) {
                    let _ = tx.try_send(ev);
                }
            }
            // Unknown notification: ignore. v1 is intentionally narrow.
        }
        Message::Response { error: Some(e), .. } => {
            let _ = tx.try_send(AgentEvent::Error {
                code: error_code(&e),
                message: e.message,
                recoverable: e.code != -32001,
            });
        }
        Message::Response { error: None, .. } => {
            // Successful response with no notification payload; we
            // already route the `result` to the oneshot in the
            // JSON-RPC client, so this arm is a no-op.
        }
        Message::Request { .. } => {
            // Requests from the agent are handled directly in handle_message,
            // so we do not emit any AgentEvents for them.
        }
    }
}

fn error_code(e: &RpcError) -> String {
    match e.code {
        -32001 => "agent_died".into(),
        -32601 => "method_not_found".into(),
        -32602 => "invalid_params".into(),
        -32603 => "internal".into(),
        _ => "rpc_error".into(),
    }
}

fn stop_reason_from_str(s: &str) -> crate::domain::agent_event::StopReason {
    use crate::domain::agent_event::StopReason::*;
    match s {
        "end_of_turn" | "end_turn" | "endTurn" | "end" => EndOfTurn,
        "cancelled" | "canceled" => Cancelled,
        "max_tokens" => MaxTokens,
        _ => Error,
    }
}

fn stop_reason_from_error(e: &RpcError) -> crate::domain::agent_event::StopReason {
    if e.code == -32001 {
        crate::domain::agent_event::StopReason::Error
    } else {
        crate::domain::agent_event::StopReason::Error
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_kind_matches_static() {
        let r = AcpRuntime::new("opencode", "curl -fsSL https://opencode.ai/install | bash");
        assert_eq!(r.kind(), "opencode");
        assert!(r.install_command().contains("opencode.ai"));
    }

    #[test]
    fn stop_reason_parses_known_strings() {
        use crate::domain::agent_event::StopReason::*;
        assert!(matches!(stop_reason_from_str("end_of_turn"), EndOfTurn));
        assert!(matches!(stop_reason_from_str("end_turn"), EndOfTurn));
        assert!(matches!(stop_reason_from_str("endTurn"), EndOfTurn));
        assert!(matches!(stop_reason_from_str("cancelled"), Cancelled));
        assert!(matches!(stop_reason_from_str("max_tokens"), MaxTokens));
        assert!(matches!(stop_reason_from_str("anything_else"), Error));
    }

    #[test]
    fn error_code_maps_known_codes() {
        let e = RpcError { code: -32001, message: "x".into(), data: None };
        assert_eq!(error_code(&e), "agent_died");
        let e = RpcError { code: -32603, message: "x".into(), data: None };
        assert_eq!(error_code(&e), "internal");
    }
}
