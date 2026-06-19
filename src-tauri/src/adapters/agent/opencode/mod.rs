//! `anomalyco/opencode` agent — the open-source coding agent. Demeteo's
//! integration targets this project; we are not affiliated with it.
//!
//! Wire format: `opencode run --format json` emits nd-JSON on stdout.
//! The prompt is passed via stdin to avoid OS ARG_MAX limits.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader, Read};

use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::domain::models::SessionInfo;
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};
use crate::ports::execution::InteractiveHandle;

pub const OPENCODE_INSTALL: &str = "curl -fsSL https://opencode.ai/install | bash";

/// Parse an opencode CLI JSON-lines event into an `AgentEvent`.
///
/// The `--format json` output uses two possible shapes:
///
/// 1. **Top-level `kind`** (Anthropic/Claude-style, most common):
///    ```json
///    {"type":"text","delta":"hello "}
///    {"type":"tool_call","name":"Read","input":{...}}
///    {"type":"usage","inputTokens":100,"outputTokens":50,"costUsd":0.001}
///    ```
///
/// 2. **Nested `update.sessionUpdate`** (opencode ACP server style):
///    ```json
///    {"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"..."}}}
///    ```
///
/// Unknown event types are silently dropped so future agent versions
/// don't break the stream.
pub fn parse_opencode_event(line: &str) -> Option<AgentEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;

    // Shape 0 (current opencode v1+ wire format):
    //   { "type": "...", "timestamp": ..., "sessionID": "...", "part": { ... } }
    // The top-level `type` discriminates; the payload is nested under `part`.
    // This is what `opencode run --format json` actually emits as of 2025-2026.
    if let Some(kind) = v.get("type").or_else(|| v.get("kind")).and_then(|t| t.as_str()) {
        // Prefer the new shape — it has the actual data. Fall through to the
        // legacy shapes only if this one explicitly returned `None` *and* the
        // shape markers for a legacy format are present (the legacy
        // discriminators carry data inline, not in `part`).
        if let Some(evt) = parse_part_shape(kind, &v) {
            return Some(evt);
        }
        // Heuristic: if the line has no `part` field at all, treat it as a
        // legacy shape where the data sits at the top level.
        if v.get("part").is_none() {
            if let Some(evt) = parse_top_level_kind(kind, &v) {
                return Some(evt);
            }
        }
    }

    // Shape 2: nested `update.sessionUpdate` (opencode ACP server style)
    if let Some(update) = v.get("update") {
        if let Some(discriminator) = update.get("sessionUpdate").and_then(|v| v.as_str()) {
            return parse_nested_session_update(discriminator, update);
        }
    }

    None
}

/// Parse the current opencode v1+ wire format. The top-level `kind` is the
/// discriminator; the payload lives in `part`. We pick the most-important
/// event to emit from each line:
///   - `text`         → `Text { delta: part.text }`
///   - `step_finish` with `part.reason == "stop"`         → `TurnComplete`
///   - `step_finish` with `part.reason == "tool-calls"`  → `Usage` (so the
///     per-step cost is captured at intermediate steps; the final "stop"
///     emits `TurnComplete` instead, and the parser signature only allows
///     one event per line)
///   - `step_finish` with no reason / unknown reason       → drop
///   - `tool_use`     → `Text { delta: <formatted line> }` (we don't have
///     permission-gated tool bridging in v1, so just dump it for the text
///     artifact and the UI stream)
///   - `step_start`, `snapshot`, unknown kinds             → drop
fn parse_part_shape(kind: &str, v: &serde_json::Value) -> Option<AgentEvent> {
    let part = v.get("part")?;
    match kind {
        "text" => {
            let text = part
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            if text.is_empty() {
                None
            } else {
                Some(AgentEvent::Text { delta: text })
            }
        }
        "step_finish" => {
            let reason = part.get("reason").and_then(|r| r.as_str()).unwrap_or("");
            if reason == "stop" {
                Some(AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                })
            } else if let Some(tokens) = part.get("tokens") {
                let input_tokens = tokens.get("input").and_then(|t| t.as_u64()).unwrap_or(0);
                let output_tokens = tokens.get("output").and_then(|t| t.as_u64()).unwrap_or(0);
                let cost_usd = part.get("cost").and_then(|t| t.as_f64());
                Some(AgentEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cost_usd,
                })
            } else {
                None
            }
        }
        "tool_use" => {
            // Format tool calls for the text artifact. Only completed tools
            // with output are included — error/running tools are noise that
            // would pollute the artifact and confuse downstream steps (e.g.
            // a spec step reading a research artifact full of "[tool read
            // status=error]" breadcrumbs).
            let tool = part
                .get("tool")
                .and_then(|t| t.as_str())
                .unwrap_or("unknown");
            let call_id = part
                .get("callID")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let state = part.get("state");
            let status = state
                .and_then(|s| s.get("status"))
                .and_then(|t| t.as_str())
                .unwrap_or("running");
            let input = state
                .and_then(|s| s.get("input"))
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let output = state
                .and_then(|s| s.get("output"))
                .and_then(|t| t.as_str())
                .unwrap_or("");

            if status == "completed" {
                let mut line = format!(
                    "[tool {tool} id={call_id}]"
                );
                if !output.is_empty() {
                    line.push_str(&format!("\n{output}"));
                } else {
                    let input_str = serde_json::to_string(&input).unwrap_or_default();
                    line.push_str(&format!("\ninput: {input_str}"));
                }
                Some(AgentEvent::Text { delta: line })
            } else {
                eprintln!(
                    "[opencode tool] {tool} ({status}) id={call_id}"
                );
                None
            }
        }
        "step_start" | "snapshot" | "patch" => None,
        _ => None,
    }
}

fn parse_top_level_kind(kind: &str, v: &serde_json::Value) -> Option<AgentEvent> {
    match kind {
        "text" | "message" | "assistant" | "text_delta" => {
            let delta = v.get("delta")
                .or_else(|| v.get("content"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if delta.is_empty() { None } else { Some(AgentEvent::Text { delta }) }
        }
        "tool_call" | "tool_use" => {
            let tool = v.get("name").or_else(|| v.get("tool")).and_then(|v| v.as_str()).unwrap_or("unknown");
            let input = serde_json::to_string(&v.get("input").or_else(|| v.get("data")).unwrap_or(&serde_json::Value::Null)).ok()?;
            Some(AgentEvent::Text { delta: format!("[tool: {}] {}", tool, input) })
        }
        "usage" | "usage_update" => {
            let input_tokens = v.get("inputTokens").or_else(|| v.get("input_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            let output_tokens = v.get("outputTokens").or_else(|| v.get("output_tokens")).and_then(|v| v.as_u64()).unwrap_or(0);
            let cost_usd = v.get("costUsd").or_else(|| v.get("cost_usd")).and_then(|v| v.as_f64());
            Some(AgentEvent::Usage { input_tokens, output_tokens, cost_usd })
        }
        "plan" => {
            let entries = serde_json::from_value(
                v.get("entries").or_else(|| v.get("steps")).cloned().unwrap_or_default()
            ).ok().unwrap_or_default();
            Some(AgentEvent::Plan { entries })
        }
        "end_turn" | "message_stop" | "done" => {
            Some(AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn })
        }
        "error" => {
            let message = v.get("message").or_else(|| v.get("error")).and_then(|v| v.as_str()).unwrap_or("unknown").to_string();
            Some(AgentEvent::Error { code: "cli_error".to_string(), message, recoverable: false })
        }
        _ => None,
    }
}

fn parse_nested_session_update(discriminator: &str, update: &serde_json::Value) -> Option<AgentEvent> {
    match discriminator {
        "agent_message_chunk" => {
            let delta = update.get("content")
                .and_then(|c| c.get("text"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if delta.is_empty() { None } else { Some(AgentEvent::Text { delta }) }
        }
        "tool_call" => {
            let tool_call_id = update.get("toolCallId").or_else(|| update.get("id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let action_str = update.get("action").or_else(|| update.get("name")).and_then(|v| v.as_str()).unwrap_or("unknown");
            let target = update.get("path").or_else(|| update.get("target")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let preview = update.get("input").or_else(|| update.get("rawInput")).map(|v| v.to_string());
            let action = serde_json::from_str::<crate::domain::action::ActionKind>(action_str).ok()?;
            let intercept_id = format!("oc-{}", tool_call_id);
            Some(AgentEvent::ToolCall { tool_call_id, intercept_id, action, target, preview })
        }
        "tool_call_update" => {
            let tool_call_id = update.get("toolCallId").or_else(|| update.get("id")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let status = if let Some(status_val) = update.get("status").or_else(|| update.get("state")).and_then(|v| v.as_str()) {
                match status_val {
                    "completed" => crate::domain::agent_event::ToolCallStatus::Completed,
                    "failed" => crate::domain::agent_event::ToolCallStatus::Failed {
                        reason: update.get("reason").or_else(|| update.get("error")).and_then(|v| v.as_str()).unwrap_or("").to_string()
                    },
                    "in_progress" => crate::domain::agent_event::ToolCallStatus::InProgress { message: None },
                    _ => crate::domain::agent_event::ToolCallStatus::Pending,
                }
            } else {
                crate::domain::agent_event::ToolCallStatus::Pending
            };
            let preview = update.get("preview").map(|v| v.to_string());
            Some(AgentEvent::ToolCallUpdate { tool_call_id, status, preview })
        }
        "usage_update" => {
            let input_tokens = update.get("inputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let output_tokens = update.get("outputTokens").and_then(|v| v.as_u64()).unwrap_or(0);
            let cost_usd = update.get("costUsd").and_then(|v| v.as_f64());
            Some(AgentEvent::Usage { input_tokens, output_tokens, cost_usd })
        }
        "plan" => {
            let entries = serde_json::from_value(
                update.get("entries").cloned().unwrap_or_default()
            ).ok().unwrap_or_default();
            Some(AgentEvent::Plan { entries })
        }
        "current_mode_update" => {
            let mode_id = update.get("mode").or_else(|| update.get("modeId")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            if mode_id.is_empty() { None } else { Some(AgentEvent::ModeChanged { mode_id }) }
        }
        "agent_thought_chunk" | "available_commands_update" | "session_info_update" => None,
        _ => None,
    }
}

/// `std::io::Read` adapter over a `Box<dyn InteractiveHandle>`. The handle's
/// native API is byte-oriented (`try_read`); wrapping it lets us drive a
/// `BufReader::read_line` loop, which is the only way to safely reassemble
/// nd-JSON lines that may straddle read boundaries.
///
/// Each `read` call briefly locks the inner mutex, so the reader thread and
/// the session's `kill()`/`Drop` never deadlock — the lock is held only for
/// the duration of one `try_read` syscall.
struct HandleReader {
    handle: Arc<Mutex<Box<dyn InteractiveHandle>>>,
}

impl Read for HandleReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let h = self.handle.lock().expect("HandleReader mutex poisoned");
        h.try_read(buf)
    }
}

/// Drain a nd-JSON stream from `reader`, parse each line with `parse_event`,
/// and forward events to `tx`. A line that arrives split across two `read`
/// calls is correctly reassembled by the underlying `BufReader` — this is
/// the whole reason the reader was rewritten (the previous chunk-based
/// `String::from_utf8_lossy(...).lines()` parser dropped any event whose
/// JSON straddled a read boundary).
///
/// On EOF, if no terminal event (`TurnComplete` / `Error`) was emitted by
/// the agent itself, `exit_code_fn` is consulted:
/// - `None` or `Some(0)` → emit a synthetic `TurnComplete { EndOfTurn }`
/// - `Some(non-zero)`   → emit `Error { code: "agent_exit_nonzero", ... }`
///
/// The function returns early if the consumer drops the `tx`.
fn drain_lines<R, F>(
    reader: R,
    parse_event: fn(&str) -> Option<AgentEvent>,
    exit_code_fn: F,
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
    session_capture: Option<Arc<Mutex<Option<String>>>>,
) where
    R: Read,
    F: FnOnce() -> Option<i32>,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut terminal = false;
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                // Capture opencode's session ID from the first JSON output
                // line that carries one. This is used for --session/--continue
                // on subsequent prompts within the same feature run.
                if let Some(ref capture) = session_capture {
                    if let Ok(guard) = capture.lock() {
                        if guard.is_none() {
                            drop(guard);
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                if let Some(sid) = v.get("sessionID").and_then(|s| s.as_str()) {
                                    if let Ok(mut g) = capture.lock() {
                                        *g = Some(sid.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(evt) = parse_event(trimmed) {
                    let is_terminal = matches!(
                        evt,
                        AgentEvent::TurnComplete { .. } | AgentEvent::Error { .. }
                    );
                    if tx.blocking_send(evt).is_err() {
                        return;
                    }
                    if is_terminal {
                        terminal = true;
                        break;
                    }
                }
            }
        }
    }
    if !terminal {
        match exit_code_fn() {
            Some(0) | None => {
                let _ = tx.blocking_send(AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                });
            }
            Some(code) => {
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "agent_exit_nonzero".to_string(),
                    message: format!("opencode exited with code {}", code),
                    recoverable: false,
                });
            }
        }
    }
}

/// opencode CLI-specific session that passes the prompt via stdin.
/// The process is spawned on first `prompt()` call since the
/// prompt is needed to build the command line.
struct OpencodeAgentSession {
    session_id: String,
    /// Resolved absolute path to the binary, resolved once in `start()`.
    /// For local machines this is the result of `which opencode`.
    /// For remote machines this is just the binary name (resolved over SSH at spawn time).
    resolved_binary: Option<String>,
    ctx: AgentContext,
    parse_event: fn(&str) -> Option<AgentEvent>,
    /// Live local opencode child, if `prompt()` was called on a `local` /
    /// empty `machine_id` and the process hasn't been reaped yet. Populated
    /// by `spawn_local`, cleared by the reader thread when it finishes
    /// naturally — or by `kill()` / `Drop` if the consumer tears down early.
    /// Only one of `live_local` / `live_remote` is `Some` at a time.
    live_local: Mutex<Option<Arc<Mutex<std::process::Child>>>>,
    /// Live remote opencode handle (SSH), if `prompt()` was called on a
    /// non-local `machine_id`. Same ownership semantics as `live_local`.
    live_remote: Mutex<Option<Arc<Mutex<Box<dyn InteractiveHandle>>>>>,
    /// Session ID captured from opencode's first response. On the very first
    /// `prompt()` call the CLI is invoked without `--session` (letting opencode
    /// create its own session), and the `sessionID` field from the first JSON
    /// output line is captured here. Subsequent `prompt()` calls pass
    /// `--session <captured> --continue` for cross-step context continuity.
    captured_session_id: Arc<Mutex<Option<String>>>,
    /// Heartbeat updated by the stderr drain thread; polled by the step
    /// executor to distinguish "working" from "blocked".
    stderr_hb: crate::ports::agent_runtime::StderrHeartbeat,
}

impl OpencodeAgentSession {
    fn build_command(&self) -> Command {
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);
        let mut cmd = Command::new(binary);
        cmd.arg("run").arg("--format").arg("json");
        // Session continuity: only pass --session/--continue when we
        // have a captured session ID from a prior prompt call. The
        // first prompt in a feature run invokes opencode without
        // --session, letting it create its own session — the sessionID
        // is captured from the first JSON output line and stored in
        // `captured_session_id`. Subsequent prompts continue that
        // session with --session <id> --continue.
        if let Ok(guard) = self.captured_session_id.lock() {
            if let Some(ref captured) = *guard {
                cmd.arg("--session").arg(captured.as_str());
                cmd.arg("--continue");
            }
        }
        if let Some(ref m) = self.ctx.model {
            cmd.arg("--model").arg(m);
        }
        if let Some(ref title) = self.ctx.title {
            cmd.arg("--title").arg(title);
        }
        cmd.arg("--dir").arg(&self.ctx.cwd);
        // Inject env vars from the context (OPENCODE_PERMISSION, etc.)
        let envs = self.ctx.env.clone();
        // Prompt is delivered via stdin to avoid ARG_MAX when
        // attached artifacts make the prompt very large.
        cmd.current_dir(&self.ctx.cwd);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in &envs {
            cmd.env(k, v);
        }
        cmd
    }
}

impl AgentSession for OpencodeAgentSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        // Defensive: in the opencode one-shot model, `prompt()` is called
        // exactly once per session. If a prior call somehow left a live
        // process (consumer dropped the stream mid-turn, or a stale
        // session was reused), reap it before starting a new one so we
        // don't leak processes.
        self.kill_live_local();
        self.kill_live_remote();

        let parse_event = self.parse_event;
        let is_local = self.ctx.machine_id.is_empty() || self.ctx.machine_id == "local";
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);

        if is_local {
            self.spawn_local(text, parse_event, tx);
        } else {
            self.spawn_remote(text, parse_event, tx);
        }

        Box::pin(ReceiverStream::new(rx))
    }

    fn cancel(&self) -> Result<(), String> {
        self.kill()
    }

    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        Ok(())
    }

    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo::default()
    }

    fn kill(&self) -> Result<(), String> {
        self.kill_live_local();
        self.kill_live_remote();
        Ok(())
    }

    fn stderr_heartbeat(&self) -> Option<crate::ports::agent_runtime::StderrHeartbeat> {
        Some(self.stderr_hb.clone())
    }
}

impl OpencodeAgentSession {
    fn build_args(&self) -> Vec<String> {
        let mut args = vec!["run".to_string(), "--format".to_string(), "json".to_string()];
        if let Ok(guard) = self.captured_session_id.lock() {
            if let Some(ref captured) = *guard {
                args.push("--session".to_string());
                args.push(captured.as_str().to_string());
                args.push("--continue".to_string());
            }
        }
        if let Some(ref m) = self.ctx.model {
            args.push("--model".to_string());
            args.push(m.clone());
        }
        if let Some(ref title) = self.ctx.title {
            args.push("--title".to_string());
            args.push(title.clone());
        }
        args.push("--dir".to_string());
        args.push(self.ctx.cwd.clone());
        // Prompt is delivered via stdin, not as a positional arg.
        args
    }

    fn spawn_local(
        &self,
        text: &str,
        parse_event: fn(&str) -> Option<AgentEvent>,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) {
        let mut cmd = self.build_command();
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::NotFound {
                    format!("opencode binary not found at '{}'. Install: {}", binary, OPENCODE_INSTALL)
                } else {
                    format!("failed to spawn opencode ({}): {}", binary, e)
                };
                let _ = tx.try_send(AgentEvent::Error { code: "spawn_failed".to_string(), message: msg, recoverable: false });
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take();

        // Write the prompt to stdin in a dedicated thread so the process
        // can consume it concurrently with stdout/stderr draining.
        // This avoids the OS ARG_MAX limit that was hit when the prompt
        // (with attached artifacts) was passed as a positional argument.
        if let Some(mut stdin) = child.stdin.take() {
            let text_owned = text.to_string();
            std::thread::spawn(move || {
                use std::io::Write;
                let _ = stdin.write_all(text_owned.as_bytes());
                let _ = stdin.write_all(b"\n");
                let _ = stdin.flush();
                // stdin is dropped here, signaling EOF to opencode
            });
        }

        // Wrap the child in Arc<Mutex<>> so the reader thread can query
        // its exit code at EOF AND the session's `kill()` / `Drop` can
        // reap it if the consumer tears down early.
        let child = Arc::new(Mutex::new(child));
        if let Ok(mut guard) = self.live_local.lock() {
            *guard = Some(child.clone());
        }

        if let Some(stderr) = stderr {
            let hb = self.stderr_hb.clone();
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(l) = line {
                        let trimmed = l.trim();
                        if !trimmed.is_empty() {
                            eprintln!("[opencode stderr] {}", trimmed);
                            hb.beat();
                        }
                    }
                }
            });
        }

        let exit_child = child.clone();
        let exit_code_fn = move || -> Option<i32> {
            exit_child
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok().flatten())
                .and_then(|status| status.code())
        };

        let session_capture = self.captured_session_id.clone();

        std::thread::spawn(move || {
            drain_lines(BufReader::new(stdout), parse_event, exit_code_fn, tx, Some(session_capture));
            // Process has been reaped. Clear the live entry so Drop /
            // kill() don't try to kill an already-exited child.
            // (Best-effort: the lock may be contended with an in-flight
            // kill — that's fine, kill() will see None and no-op.)
        });
    }

    fn spawn_remote(&self, text: &str, parse_event: fn(&str) -> Option<AgentEvent>, tx: tokio::sync::mpsc::Sender<AgentEvent>) {
        let args = self.build_args();
        let machine_id = self.ctx.machine_id.clone();
        let binary = self.ctx.binary.clone();
        let cwd = self.ctx.cwd.clone();
        let env = self.ctx.env.clone();
        let exec = self.ctx.exec.clone();

        // Spawn the remote opencode synchronously in the current thread so
        // spawn failures (auth, network, PTY allocation) surface as
        // `AgentEvent::Error` on the stream — not as a silent "the stream
        // just never produces anything", which is what the previous
        // post-hoc spawn inside the reader thread could fall into if the
        // channel was already closed.
        let handle = match exec.spawn_interactive(&machine_id, &binary, &args, &cwd, &env) {
            Ok(h) => h,
            Err(e) => {
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: format!("failed to spawn opencode over SSH: {}", e),
                    recoverable: false,
                });
                return;
            }
        };

        // Deliver the prompt via stdin to avoid ARG_MAX on the SSH command line
        let _ = handle.write_line(text);

        let handle = Arc::new(Mutex::new(handle));
        if let Ok(mut guard) = self.live_remote.lock() {
            *guard = Some(handle.clone());
        }

        let exit_handle = handle.clone();
        let exit_code_fn = move || -> Option<i32> {
            exit_handle
                .lock()
                .ok()
                .and_then(|mut h| h.try_wait().ok().flatten())
        };

        // Drive the line-buffered reader in a dedicated thread. The
        // `HandleReader` locks the inner mutex only for the duration of
        // each `try_read` syscall, so the session's `kill()` / `Drop` can
        // still acquire the lock to send the SIGKILL to the remote agent.
        let reader = HandleReader { handle: handle.clone() };
        let session_capture = self.captured_session_id.clone();
        std::thread::spawn(move || {
            drain_lines(reader, parse_event, exit_code_fn, tx, Some(session_capture));
            // Reader thread finished; the remote opencode has exited and
            // been reaped. The live entry in `self.live_remote` will be
            // cleared by the next `prompt()` or by `kill()` / `Drop`.
        });
    }

    /// Take the local child (if any) out of `live_local` and reap it.
    /// Safe to call concurrently with the reader thread; whichever side
    /// wins the `take()` does the work, the other side sees `None`.
    fn kill_live_local(&self) {
        let child = match self.live_local.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(child) = child else { return };
        let Ok(mut c) = child.lock() else { return };
        // If the reader thread already finished naturally, the child has
        // been waited on and is a zombie. `try_wait` returns `Some(_)`
        // and we just `wait()` to reap without sending another signal.
        // If the reader thread is still running, the child is still
        // alive and we kill it.
        match c.try_wait().ok().flatten() {
            Some(_) => {
                let _ = c.wait();
            }
            None => {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
    }

    /// Take the remote handle (if any) out of `live_remote` and kill the
    /// remote opencode process. The SSH channel's `kill()` closes the
    /// channel; the agent process on the remote host is reaped by its
    /// own kernel once the SSH session ends.
    fn kill_live_remote(&self) {
        let arc = match self.live_remote.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(arc) = arc else { return };
        let mut h = match arc.lock() {
            Ok(h) => h,
            Err(_) => return,
        };
        let _ = h.kill();
    }
}

impl Drop for OpencodeAgentSession {
    fn drop(&mut self) {
        // Reap any live process on the way out. Without this, a consumer
        // that drops the stream before EOF would leave opencode running
        // indefinitely (and holding the worktree / lock files).
        self.kill_live_local();
        self.kill_live_remote();
    }
}

/// Runtime for opencode CLI mode.
///
/// Like `CliAgentRuntime`, the prompt is delivered via stdin. The process is
/// spawned on the first `prompt()` call since the prompt is needed to
/// construct the CLI args.
pub struct OpencodeCliRuntime {
    inner: super::cli_runtime::CliAgentRuntime,
}

impl OpencodeCliRuntime {
    pub fn new() -> Self {
        Self {
            inner: super::cli_runtime::CliAgentRuntime {
                kind_str: "opencode",
                binary: "opencode",
                extra_args: &["run", "--format", "json"],
                install_cmd: OPENCODE_INSTALL,
                parse_event: parse_opencode_event as fn(&str) -> Option<AgentEvent>,
            },
        }
    }
}

impl Default for OpencodeCliRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentRuntime for OpencodeCliRuntime {
    fn kind(&self) -> &'static str {
        "opencode"
    }

    fn is_available(&self, exec: &dyn crate::ports::execution::ExecutionPort, machine_id: &str) -> bool {
        self.inner.is_available(exec, machine_id)
    }

    fn install_command(&self) -> &'static str {
        self.inner.install_command()
    }

    fn start(
        &self,
        ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>> {
        Box::pin(async move {
            // Resolve the binary path once at spawn time. For local machines,
            // use the resolved absolute path so spawn always works even if PATH
            // changes between availability check and prompt. For remote machines,
            // leave None and use the binary name as-is (resolved over SSH).
            let resolved_binary = if ctx.machine_id.is_empty() || ctx.machine_id == "local" {
                super::resolve_local_binary_path(&ctx.binary)
            } else {
                None
            };
            let session = OpencodeAgentSession {
                session_id: format!("opencode-{}", ctx.thread_id),
                resolved_binary,
                ctx,
                parse_event: parse_opencode_event,
                live_local: Mutex::new(None),
                live_remote: Mutex::new(None),
                captured_session_id: Arc::new(Mutex::new(None)),
                stderr_hb: crate::ports::agent_runtime::StderrHeartbeat::new(),
            };
            Ok(Arc::new(session) as Arc<dyn AgentSession>)
        })
    }
}

pub fn runtime() -> OpencodeCliRuntime {
    OpencodeCliRuntime::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tokio::sync::mpsc;

    /// Run `drain_lines` in a background thread with the given reader and
    /// exit-code closure; collect the events it emits into a `Vec` for
    /// assertion. Drains the channel until it closes.
    fn run_drain<R, F>(reader: R, exit_code_fn: F) -> Vec<AgentEvent>
    where
        R: Read + Send + 'static,
        F: FnOnce() -> Option<i32> + Send + 'static,
    {
        let (tx, mut rx) = mpsc::channel::<AgentEvent>(64);
        let parse_event: fn(&str) -> Option<AgentEvent> = parse_opencode_event;
        std::thread::spawn(move || {
            drain_lines(reader, parse_event, exit_code_fn, tx, None);
        });
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut events = Vec::new();
            while let Some(e) = rx.recv().await {
                events.push(e);
            }
            events
        })
    }

    /// The bug that started this work: the previous chunk-based reader
    /// (`String::from_utf8_lossy(&buf[..n]).lines()`) parsed each read
    /// independently, so any JSON event whose bytes straddled two reads
    /// was lost. This test splits a single event mid-value and confirms
    /// the new line-buffered reader reassembles it correctly.
    #[test]
    fn drain_lines_reassembles_event_split_across_two_reads() {
        let full = br#"{"type":"text","delta":"hello world"}
{"type":"end_turn"}
"#;
        // Split inside the JSON value, before the closing `}`.
        let split_at = 18;
        let (c1, c2) = full.split_at(split_at);
        let reader = Cursor::new(c1.to_vec()).chain(Cursor::new(c2.to_vec()));

        let events = run_drain(reader, || Some(0));
        assert_eq!(events.len(), 2, "got: {:?}", events);
        match &events[0] {
            AgentEvent::Text { delta } => assert_eq!(delta, "hello world"),
            e => panic!("expected Text, got {:?}", e),
        }
        match &events[1] {
            AgentEvent::TurnComplete { .. } => {}
            e => panic!("expected TurnComplete, got {:?}", e),
        }
    }

    #[test]
    fn drain_lines_handles_multiple_events_in_one_read() {
        let full = br#"{"type":"text","delta":"a"}
{"type":"text","delta":"b"}
{"type":"end_turn"}
"#;
        let events = run_drain(Cursor::new(full.to_vec()), || Some(0));
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "a"));
        assert!(matches!(&events[1], AgentEvent::Text { delta } if delta == "b"));
        assert!(matches!(&events[2], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_emits_error_on_nonzero_exit() {
        let reader = Cursor::new(br#"{"type":"text","delta":"x"}"#.to_vec());
        let events = run_drain(reader, || Some(137));
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "x"));
        match &events[1] {
            AgentEvent::Error { message, .. } => {
                assert!(message.contains("137"), "got: {}", message);
            }
            e => panic!("expected Error, got {:?}", e),
        }
    }

    #[test]
    fn drain_lines_emits_turn_complete_on_zero_exit_when_empty() {
        // Agent produced no output, exited cleanly. Previously this case
        // also worked (a synthetic TurnComplete was always emitted), but
        // the new exit-code path is the one doing the work.
        let events = run_drain(Cursor::new(Vec::new()), || Some(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_emits_error_when_empty_and_nonzero_exit() {
        // The critical case the old code masked: agent crashed with no
        // output. Old code emitted a synthetic TurnComplete and the step
        // was marked completed; new code surfaces it as an Error.
        let events = run_drain(Cursor::new(Vec::new()), || Some(1));
        assert_eq!(events.len(), 1);
        match &events[0] {
            AgentEvent::Error { message, .. } => assert!(message.contains("1")),
            e => panic!("expected Error, got {:?}", e),
        }
    }

    #[test]
    fn drain_lines_skips_garbage_lines() {
        // Non-JSON lines (e.g. stray warnings on stdout) are dropped by
        // `parse_event`; valid events around them still flow through.
        let full = b"this is not json\n{\"type\":\"end_turn\"}\n";
        let events = run_drain(Cursor::new(full.to_vec()), || Some(0));
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_stops_at_terminal_event_even_if_more_data_pending() {
        // Trailing data after a terminal event must not produce a second
        // TurnComplete or any other event.
        let full = br#"{"type":"text","delta":"final"}
{"type":"end_turn"}
{"type":"text","delta":"this should be dropped"}
"#;
        let events = run_drain(Cursor::new(full.to_vec()), || Some(0));
        assert_eq!(events.len(), 2, "got: {:?}", events);
        assert!(matches!(&events[0], AgentEvent::Text { delta } if delta == "final"));
        assert!(matches!(&events[1], AgentEvent::TurnComplete { .. }));
    }

    #[test]
    fn drain_lines_returns_early_when_consumer_drops() {
        // If the consumer side is dropped, subsequent `blocking_send`s
        // fail and `drain_lines` must return without panicking.
        let (tx, rx) = mpsc::channel::<AgentEvent>(1);
        drop(rx);
        let reader = Cursor::new(
            br#"{"type":"text","delta":"a"}
{"type":"text","delta":"b"}
{"type":"end_turn"}
"#
            .to_vec(),
        );
        let parse_event: fn(&str) -> Option<AgentEvent> = parse_opencode_event;
        drain_lines(reader, parse_event, || Some(0), tx, None);
    }

    /// Mock `InteractiveHandle` that returns pre-canned byte chunks one
    /// `try_read` call at a time. Drives the real `HandleReader` adapter
    /// to confirm the SSH path is also fixed.
    struct ChunkyHandle {
        chunks: std::sync::Mutex<Vec<Vec<u8>>>,
        exit_code: i32,
    }
    impl ChunkyHandle {
        fn new(chunks: Vec<&[u8]>, exit_code: i32) -> Self {
            Self {
                chunks: std::sync::Mutex::new(
                    chunks.into_iter().map(<[u8]>::to_vec).collect(),
                ),
                exit_code,
            }
        }
    }
    impl InteractiveHandle for ChunkyHandle {
        fn write_line(&self, _: &str) -> std::io::Result<usize> {
            Ok(0)
        }
        fn try_read(&self, buf: &mut [u8]) -> std::io::Result<usize> {
            let mut q = self.chunks.lock().unwrap();
            match q.first() {
                Some(chunk) => {
                    let n = chunk.len().min(buf.len());
                    buf[..n].copy_from_slice(&chunk[..n]);
                    if n == chunk.len() {
                        q.remove(0);
                    } else {
                        q[0] = q[0].split_off(n);
                    }
                    Ok(n)
                }
                None => Ok(0),
            }
        }
        fn kill(&self) -> Result<(), String> {
            Ok(())
        }
        fn try_wait(&self) -> Result<Option<i32>, String> {
            Ok(Some(self.exit_code))
        }
    }

    #[test]
    fn handle_reader_reassembles_split_line_via_try_read() {
        // SSH-path regression test: bytes arrive in arbitrary-sized chunks
        // (one `try_read` per chunk here), the line buffer must reassemble.
        let handle = Arc::new(Mutex::new(Box::new(ChunkyHandle::new(
            vec![
                b"{\"type\":\"text\",\"de",    // 19 bytes, partial line
                b"lta\":\"split\"}\n",          // completes the line
                b"{\"type\":\"end_turn\"}\n",   // terminal event
            ],
            0,
        )) as Box<dyn InteractiveHandle>));
        let handle_for_exit = handle.clone();
        let reader = HandleReader { handle };
        let events = run_drain(reader, move || {
            handle_for_exit
                .lock()
                .ok()
                .and_then(|mut h| h.try_wait().ok().flatten())
        });
        assert_eq!(events.len(), 2, "got: {:?}", events);
        match &events[0] {
            AgentEvent::Text { delta } => assert_eq!(delta, "split"),
            e => panic!("expected Text, got {:?}", e),
        }
        assert!(matches!(&events[1], AgentEvent::TurnComplete { .. }));
    }

    // ── parse_opencode_event: current opencode v1+ wire format ───────────
    //
    // The shape the binary actually emits is:
    //   { "type": "...", "timestamp": ..., "sessionID": "...",
    //     "part": { "type": "...", "text"|"tool"|"reason"|"tokens"|... } }
    // The legacy heuristics in `parse_top_level_kind` /
    // `parse_nested_session_update` never match this shape, which is why
    // every event was being dropped before the parser fix. These tests
    // pin the new behaviour.

    #[test]
    fn parse_event_text_uses_part_text() {
        // Real shape from `opencode run --format json` (2025-2026 binary).
        let line = r#"{"type":"text","timestamp":1234,"sessionID":"s1","part":{"id":"p1","messageID":"m1","sessionID":"s1","type":"text","text":"hello world","time":{"start":1,"end":2}}}"#;
        let evt = parse_opencode_event(line).expect("should parse");
        match evt {
            AgentEvent::Text { delta } => assert_eq!(delta, "hello world"),
            e => panic!("expected Text, got {:?}", e),
        }
    }

    #[test]
    fn parse_event_text_with_empty_part_text_is_dropped() {
        // The binary can emit a text event whose `part.text` is empty
        // (e.g. a chunk that just signals a state transition); drop it
        // rather than emitting a Text event with an empty delta.
        let line = r#"{"type":"text","part":{"text":""}}"#;
        assert!(parse_opencode_event(line).is_none());
    }

    #[test]
    fn parse_event_step_finish_stop_emits_turn_complete() {
        let line = r#"{"type":"step_finish","timestamp":1234,"sessionID":"s1","part":{"id":"p1","messageID":"m1","sessionID":"s1","reason":"stop","type":"step-finish","tokens":{"total":10447,"input":10408,"output":39,"reasoning":0,"cache":{"write":0,"read":0}},"cost":0}}"#;
        let evt = parse_opencode_event(line).expect("should parse");
        assert!(
            matches!(evt, AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn }),
            "got: {:?}",
            evt
        );
    }

    #[test]
    fn parse_event_step_finish_tool_calls_emits_usage() {
        // Intermediate step in a multi-step agent turn: `reason` is
        // "tool-calls" (the agent will continue). Surface the per-step
        // tokens/cost as `Usage` so cost tracking still works; the final
        // "stop" event will surface the `TurnComplete` instead.
        let line = r#"{"type":"step_finish","part":{"reason":"tool-calls","tokens":{"input":1000,"output":50,"reasoning":0,"cache":{"write":0,"read":0},"total":1050},"cost":0.002}}"#;
        let evt = parse_opencode_event(line).expect("should parse");
        match evt {
            AgentEvent::Usage { input_tokens, output_tokens, cost_usd } => {
                assert_eq!(input_tokens, 1000);
                assert_eq!(output_tokens, 50);
                assert_eq!(cost_usd, Some(0.002));
            }
            e => panic!("expected Usage, got {:?}", e),
        }
    }

    #[test]
    fn parse_event_tool_use_formats_as_text() {
        let line = r#"{"type":"tool_use","timestamp":1234,"part":{"type":"tool","tool":"bash","callID":"call_abc","state":{"status":"completed","input":{"command":"ls -la","description":"List dir"},"output":"file1\nfile2","title":"List dir","time":{"start":1,"end":2}}}}"#;
        let evt = parse_opencode_event(line).expect("should parse");
        match evt {
            AgentEvent::Text { delta } => {
                assert!(delta.contains("[tool bash"), "delta was: {}", delta);
                assert!(delta.contains("call_abc"));
                assert!(delta.contains("file1\nfile2"));
                assert!(!delta.contains("status=error"));
            }
            e => panic!("expected Text, got {:?}", e),
        }
    }

    #[test]
    fn parse_event_step_start_is_dropped() {
        let line = r#"{"type":"step_start","part":{"id":"p1","messageID":"m1","sessionID":"s1","type":"step-start"}}"#;
        assert!(parse_opencode_event(line).is_none());
    }

    #[test]
    fn parse_event_tool_use_error_is_dropped() {
        // Tool calls with status != "completed" must not produce Text
        // events — they'd pollute the artifact with noise that confuses
        // downstream steps.
        let line = r#"{"type":"tool_use","part":{"tool":"read","callID":"call_err","state":{"status":"error","input":{"filePath":"/nonexistent"},"output":"no such file","title":"Read"}}}}"#;
        assert!(parse_opencode_event(line).is_none());
    }

    #[test]
    fn parse_event_tool_use_running_is_dropped() {
        let line = r#"{"type":"tool_use","part":{"tool":"bash","callID":"call_r","state":{"status":"running","input":{"command":"ls"}}}}"#;
        assert!(parse_opencode_event(line).is_none());
    }

    #[test]
    fn parse_event_unknown_part_shape_is_dropped() {
        // Future-version event we don't know — drop silently.
        let line = r#"{"type":"some_new_event","part":{"x":1}}"#;
        assert!(parse_opencode_event(line).is_none());
    }

    #[test]
    fn parse_event_legacy_flat_text_still_works() {
        // Fallback path: a tool that emits the old top-level-delta shape
        // should still be parsed (this is what the legacy tests rely on).
        let line = r#"{"type":"text","delta":"hi"}"#;
        match parse_opencode_event(line).expect("should parse") {
            AgentEvent::Text { delta } => assert_eq!(delta, "hi"),
            e => panic!("expected Text, got {:?}", e),
        }
    }

    #[test]
    fn parse_event_legacy_flat_end_turn_still_works() {
        let line = r#"{"type":"end_turn"}"#;
        assert!(matches!(
            parse_opencode_event(line),
            Some(AgentEvent::TurnComplete { .. })
        ));
    }

    #[test]
    fn parse_event_legacy_nested_session_update_still_works() {
        // ACP-style nested shape (used by some custom opencode builds).
        let line = r#"{"update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"nested"}}}"#;
        match parse_opencode_event(line).expect("should parse") {
            AgentEvent::Text { delta } => assert_eq!(delta, "nested"),
            e => panic!("expected Text, got {:?}", e),
        }
    }

    #[test]
    fn parse_event_invalid_json_is_dropped() {
        assert!(parse_opencode_event("not json").is_none());
        assert!(parse_opencode_event("").is_none());
        assert!(parse_opencode_event("   \n").is_none());
    }

    /// End-to-end: the new wire format flows through `drain_lines` and
    /// produces the same shape of events the orchestrator's step loop
    /// expects — a sequence of `Text` events followed by a terminal
    /// `TurnComplete`.
    #[test]
    fn drain_lines_end_to_end_with_real_opencode_wire_format() {
        let input = br##"{"type":"step_start","part":{"snapshot":"snap"}}
{"type":"text","part":{"type":"text","text":"Affected Files"}}
{"type":"step_finish","part":{"type":"step-finish","reason":"tool-calls","tokens":{"input":100,"output":10,"total":110},"cost":0}}
{"type":"text","part":{"type":"text","text":"Patterns to Follow"}}
{"type":"step_finish","part":{"type":"step-finish","reason":"stop","tokens":{"input":200,"output":20,"total":220},"cost":0.001}}
"##;
        let events = run_drain(Cursor::new(input.to_vec()), || Some(0));
        // Expected: text, usage, text, turn_complete
        // (step_start dropped; the final step_finish with reason=stop
        // emits TurnComplete, the intermediate one with reason=tool-calls
        // emits Usage).
        assert_eq!(events.len(), 4, "got: {:?}", events);
        match &events[0] {
            AgentEvent::Text { delta } => assert_eq!(delta, "Affected Files"),
            e => panic!("expected Text, got {:?}", e),
        }
        match &events[1] {
            AgentEvent::Usage { input_tokens, output_tokens, .. } => {
                assert_eq!(*input_tokens, 100);
                assert_eq!(*output_tokens, 10);
            }
            e => panic!("expected Usage, got {:?}", e),
        }
        match &events[2] {
            AgentEvent::Text { delta } => assert_eq!(delta, "Patterns to Follow"),
            e => panic!("expected Text, got {:?}", e),
        }
        assert!(matches!(
            &events[3],
            AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn }
        ));
    }
}
