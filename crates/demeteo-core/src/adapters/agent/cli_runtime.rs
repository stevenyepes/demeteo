use std::future::Future;
use std::io::{BufRead, BufReader, Read};
use std::pin::Pin;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::domain::models::SessionInfo;
use crate::ports::agent_runtime::{
    AgentContext, AgentRuntime, AgentSession, AgentStartError, StderrHeartbeat,
};
use crate::ports::execution::InteractiveHandle;

/// Parse a single JSON-lines event from a CLI agent's stdout.
pub type EventParser = fn(line: &str) -> Option<AgentEvent>;

/// Construct command-line arguments for the CLI agent.
/// Build the argv for one agent invocation. The third argument is
/// the prompt the user (or orchestrator) is sending this turn; the
/// builder is responsible for placing it in whatever slot its
/// runtime expects — opencode/claude-code/hermes take it as a
/// trailing positional — rather than the agent runtime
/// `handle.write_line`-ing it after spawn, which races the
/// runtime's own `init` phase ("You must provide a message or a
/// command" on opencode). The signature makes the contract
/// explicit so a future runtime that wants a different slot has
/// to decide at build-args time, not at spawn time.
pub type ArgsBuilder =
    fn(ctx: &AgentContext, captured_session_id: Option<&str>, prompt: &str) -> Vec<String>;

/// Translate the session's [`PermissionProfile`] into agent-native
/// environment variables (e.g. opencode's `OPENCODE_PERMISSION`). Agents
/// that enforce via CLI flags instead use [`no_permission_env`] and read
/// `ctx.permissions` in their [`ArgsBuilder`].
pub type PermEnvBuilder = fn(
    p: &crate::domain::permission::PermissionProfile,
) -> std::collections::HashMap<String, String>;

/// Shared runtime for one-shot CLI-based agents (opencode, hermes, claude, etc.)
pub struct UnifiedCliRuntime {
    pub kind_str: &'static str,
    pub binary: &'static str,
    pub install_cmd: &'static str,
    pub parse_event: EventParser,
    pub build_args: ArgsBuilder,
    /// Maps the abstract permission profile to this agent's native env.
    pub perm_env: PermEnvBuilder,
    /// Human-facing name for pickers/settings (e.g. "Claude Code").
    pub display_label: &'static str,
    /// Whether `<binary> models` lists selectable models.
    pub lists_models: bool,
    /// Default model when no override is configured; `None` if not statically
    /// knowable.
    pub default_model: Option<&'static str>,
}

#[async_trait]
impl AgentRuntime for UnifiedCliRuntime {
    fn kind(&self) -> &'static str {
        self.kind_str
    }

    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: self.display_label,
            lists_models: self.lists_models,
            default_model: self.default_model,
        }
    }

    fn binary(&self) -> &'static str {
        self.binary
    }

    async fn is_available(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> bool {
        if machine_id == "local" || machine_id.is_empty() {
            super::is_binary_on_local_path(self.binary)
        } else {
            // Probe under an **interactive login** shell so the target user's
            // full environment is sourced — the login profile *and* `~/.bashrc`,
            // where developer tool-managers (`mise`/`asdf`/`nvm`) activate the
            // toolchain that puts the agent binary on `PATH`. A bare non-login
            // `command -v`, or even a non-interactive `bash -l`, misses those
            // (the `.bashrc` non-interactive guard returns first) and reports a
            // correctly-installed agent as "Missing". This must match the shell
            // mode `spawn_interactive` uses to launch the agent, so "available"
            // and "runnable" agree.
            let res = exec
                .run_command_with(
                    machine_id,
                    &format!("command -v {} >/dev/null 2>&1 && echo ok", self.binary),
                    crate::ports::execution::ShellOptions::login_interactive(),
                )
                .await;
            res.map(|out| out.trim() == "ok").unwrap_or(false)
        }
    }

    fn install_command(&self) -> &'static str {
        self.install_cmd
    }

    fn start(
        &self,
        ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>>
    {
        let kind = self.kind_str;
        let parse_event = self.parse_event;
        let build_args = self.build_args;
        let perm_env = self.perm_env;

        Box::pin(async move {
            // Translate the abstract permission profile into this agent's
            // native env. Done here (once, at spawn) so every caller only
            // has to set `ctx.permissions`; arg-based enforcement (e.g.
            // claude-code's --disallowedTools) is layered by build_args
            // reading the same `ctx.permissions`.
            let mut ctx = ctx;
            ctx.env.extend((perm_env)(&ctx.permissions));

            let resolved_binary = if ctx.machine_id.is_empty() || ctx.machine_id == "local" {
                super::resolve_local_binary_path(&ctx.binary)
            } else {
                None
            };
            let session = UnifiedCliSession {
                session_id: format!("{}-{}", kind, ctx.thread_id),
                resolved_binary,
                ctx,
                parse_event,
                build_args,
                live_local: Mutex::new(None),
                live_remote: Mutex::new(None),
                captured_session_id: Arc::new(Mutex::new(None)),
                stderr_hb: StderrHeartbeat::new(),
                cumulative_tokens: Arc::new(AtomicU64::new(0)),
            };
            Ok(Arc::new(session) as Arc<dyn AgentSession>)
        })
    }
}

#[allow(clippy::type_complexity)]
pub struct UnifiedCliSession {
    session_id: String,
    resolved_binary: Option<String>,
    ctx: AgentContext,
    parse_event: EventParser,
    build_args: ArgsBuilder,
    live_local: Mutex<Option<Arc<Mutex<std::process::Child>>>>,
    live_remote: Mutex<Option<Arc<Mutex<Box<dyn InteractiveHandle>>>>>,
    captured_session_id: Arc<Mutex<Option<String>>>,
    stderr_hb: StderrHeartbeat,
    /// Monotonic high-water mark of input + output tokens billed
    /// against this session's underlying agent process. Updated as
    /// `Usage` / `TurnComplete { usage }` events are parsed by
    /// `drain_lines`. Read by the driver's context-window watchdog
    /// via [`AgentSession::cumulative_tokens`]. Zero for a fresh
    /// session before the first event arrives.
    cumulative_tokens: Arc<AtomicU64>,
}

impl UnifiedCliSession {
    fn build_command(&self, prompt: &str) -> Command {
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);
        let mut cmd = Command::new(binary);
        let captured = {
            if let Ok(guard) = self.captured_session_id.lock() {
                guard.clone()
            } else {
                None
            }
        };
        let args = (self.build_args)(&self.ctx, captured.as_deref(), prompt);
        cmd.args(&args);
        cmd.current_dir(&self.ctx.cwd);
        // Stdin is wired to /dev/null (immediate EOF). The prompt is already
        // passed as a positional argv, so no agent reads stdin during `init`
        // or a turn; giving them a piped-but-never-written FD made opencode
        // park on a stdin read and never emit its first `created id=ses_…`
        // event (the "Waiting for agent output…" hang). /dev/null preserves
        // the no-stdin-reads contract that argv-passed agents rely on.
        cmd.stdin(Stdio::null());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in &self.ctx.env {
            cmd.env(k, v);
        }
        crate::shared::proc::sanitize_child_env(&mut cmd);
        cmd
    }

    fn spawn_local(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) {
        let mut cmd = self.build_command(text);
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let msg = if e.kind() == std::io::ErrorKind::NotFound {
                    format!("binary not found at '{}'", binary)
                } else {
                    format!("failed to spawn {} ({}): {}", self.ctx.binary, binary, e)
                };
                let _ = tx.try_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: msg,
                    recoverable: false,
                });
                return;
            }
        };

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take();
        // Stdin is `/dev/null` from `build_command`; the prompt is already
        // passed as a positional arg, so nothing is ever written here.

        let child = Arc::new(Mutex::new(child));
        if let Ok(mut guard) = self.live_local.lock() {
            *guard = Some(child.clone());
        }

        if let Some(stderr) = stderr {
            let hb = self.stderr_hb.clone();
            let kind = self.ctx.binary.clone();
            std::thread::spawn(move || {
                let reader = std::io::BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() {
                        eprintln!("[{} stderr] {}", kind, trimmed);
                        hb.beat();
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
        let cumulative = self.cumulative_tokens.clone();

        std::thread::spawn(move || {
            drain_lines(
                BufReader::new(stdout),
                parse_event,
                exit_code_fn,
                tx,
                Some(session_capture),
                Some(cumulative),
            );
        });
    }

    fn spawn_remote(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) {
        let captured = {
            if let Ok(guard) = self.captured_session_id.lock() {
                guard.clone()
            } else {
                None
            }
        };
        let args = (self.build_args)(&self.ctx, captured.as_deref(), text);
        let machine_id = self.ctx.machine_id.clone();
        let binary = self.ctx.binary.clone();
        let cwd = self.ctx.cwd.clone();
        let env = self.ctx.env.clone();
        let exec = self.ctx.exec.clone();

        let handle = match exec.spawn_interactive(&machine_id, &binary, &args, &cwd, &env) {
            Ok(h) => h,
            Err(e) => {
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: format!("failed to spawn {} over SSH: {}", self.ctx.binary, e),
                    recoverable: false,
                });
                return;
            }
        };

        let handle = Arc::new(Mutex::new(handle));
        if let Ok(mut guard) = self.live_remote.lock() {
            *guard = Some(handle.clone());
        }

        let exit_handle = handle.clone();
        let exit_code_fn = move || -> Option<i32> {
            exit_handle
                .lock()
                .ok()
                .and_then(|h| h.try_wait().ok().flatten())
        };

        let reader = HandleReader {
            handle: handle.clone(),
        };
        let session_capture = self.captured_session_id.clone();
        let cumulative = self.cumulative_tokens.clone();
        std::thread::spawn(move || {
            drain_lines(
                reader,
                parse_event,
                exit_code_fn,
                tx,
                Some(session_capture),
                Some(cumulative),
            );
        });
    }

    fn kill_live_local(&self) {
        let child = match self.live_local.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(child) = child else { return };
        let Ok(mut c) = child.lock() else { return };
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

    fn kill_live_remote(&self) {
        let arc = match self.live_remote.lock() {
            Ok(mut g) => g.take(),
            Err(_) => return,
        };
        let Some(arc) = arc else { return };
        let h = match arc.lock() {
            Ok(h) => h,
            Err(_) => return,
        };
        let _ = h.kill();
    }
}

impl AgentSession for UnifiedCliSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
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

    fn stderr_heartbeat(&self) -> Option<StderrHeartbeat> {
        Some(self.stderr_hb.clone())
    }

    fn is_alive(&self) -> bool {
        // A CLI session spawns a fresh process per turn (`prompt` kills any
        // previous one) and its resumable state is the captured session id
        // in the agent's own on-disk store — it survives process exit. So a
        // finished child does not make the session dead, and the driver's
        // dead-session respawn fallback must not churn it between steps.
        true
    }

    fn cumulative_tokens(&self) -> u64 {
        self.cumulative_tokens.load(Ordering::Relaxed)
    }
}

impl Drop for UnifiedCliSession {
    fn drop(&mut self) {
        self.kill_live_local();
        self.kill_live_remote();
    }
}

struct HandleReader {
    handle: Arc<Mutex<Box<dyn InteractiveHandle>>>,
}

impl Read for HandleReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // `try_read` reports `WouldBlock` for retryable conditions (the SSH
        // adapter maps its 10s blocking-read timeout and the keepalive-due
        // interruption to it). Retry those instead of letting `drain_lines`
        // treat them as end-of-stream — a long LLM thinking pause with no
        // wire traffic is legitimate agent behaviour; the silence/wall
        // watchdogs in `stream_agent_turn` own the "agent is stuck" call.
        // The handle mutex is re-acquired per attempt so `kill()` can grab
        // it between retries and close the channel (the next read then
        // returns EOF).
        loop {
            let res = {
                let h = self.handle.lock().expect("HandleReader mutex poisoned");
                h.try_read(buf)
            };
            match res {
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) =>
                {
                    // The SSH read blocks ~10s per attempt, so this loop is
                    // normally slow; the sleep only guards against a handle
                    // that reports WouldBlock immediately.
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                other => return other,
            }
        }
    }
}

fn drain_lines<R, F>(
    reader: R,
    parse_event: EventParser,
    exit_code_fn: F,
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
    session_capture: Option<Arc<Mutex<Option<String>>>>,
    cumulative_tokens: Option<Arc<AtomicU64>>,
) where
    R: Read,
    F: FnOnce() -> Option<i32>,
{
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut terminal = false;
    // Set when the loop ends on a read *error* rather than a clean EOF.
    // The distinction matters below: an errored stream whose process has
    // no exit status is a lost transport, not a finished turn.
    let mut read_error: Option<std::io::Error> = None;
    // Ring buffer of the last `TAIL_CAP` non-empty, unparseable lines.
    // These are the agent's stderr-ish / banner / error messages that
    // the JSON parser dropped on the floor. When the agent exits
    // non-zero the user used to see "agent exited with code 1" with
    // no context — the actual reason (e.g. "Error: provider
    // `minimax-coding-plan` not configured in /home/developer/.config/
    // opencode/opencode.json") was sitting in these lines. Surfacing
    // the tail makes the failure actionable.
    const TAIL_CAP: usize = 20;
    const TAIL_MAX_LINE: usize = 400;
    let mut tail: Vec<String> = Vec::with_capacity(TAIL_CAP);
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Err(e) => {
                read_error = Some(e);
                break;
            }
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(ref capture) = session_capture {
                    if let Ok(guard) = capture.lock() {
                        if guard.is_none() {
                            drop(guard);
                            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                                let found_sid = v
                                    .get("sessionID")
                                    .or_else(|| v.get("session_id"))
                                    .or_else(|| v.get("conversationID"))
                                    .or_else(|| v.get("conversation_id"))
                                    .or_else(|| {
                                        v.get("data").and_then(|d| d.get("conversation_id"))
                                    })
                                    .or_else(|| v.get("data").and_then(|d| d.get("session_id")))
                                    .and_then(|s| s.as_str());
                                if let Some(sid) = found_sid {
                                    if let Ok(mut g) = capture.lock() {
                                        *g = Some(sid.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(evt) = parse_event(trimmed) {
                    // Track cumulative token cost for the watchdog. Mirrors
                    // `UsageAccumulator` (monotonic-max per-field). Cache
                    // reads are included in `input_tokens` from the agent's
                    // own accounting on most providers; we treat the
                    // running input+output sum as the context-budget
                    // approximation — exact cache separation isn't needed
                    // for the 80% threshold.
                    if let Some(ref cumulative) = cumulative_tokens {
                        let delta = match &evt {
                            AgentEvent::Usage(u) => u.input_tokens + u.output_tokens,
                            AgentEvent::TurnComplete { usage: Some(u), .. } => {
                                u.input_tokens + u.output_tokens
                            }
                            _ => 0,
                        };
                        if delta > 0 {
                            let mut current = cumulative.load(Ordering::Relaxed);
                            while delta > current {
                                match cumulative.compare_exchange(
                                    current,
                                    delta,
                                    Ordering::Relaxed,
                                    Ordering::Relaxed,
                                ) {
                                    Ok(_) => break,
                                    Err(observed) => current = observed,
                                }
                            }
                        }
                    }
                    let is_terminal = matches!(
                        evt,
                        AgentEvent::TurnComplete { .. } | AgentEvent::Error { .. }
                    );
                    if tx.blocking_send(evt).is_err() {
                        return;
                    }
                    if is_terminal {
                        terminal = true;
                        // Drain remaining output to EOF before breaking so the
                        // child's write end stays open. Dropping the reader here
                        // would close the read end and trigger EPIPE in processes
                        // that keep writing after emitting a terminal event (e.g.
                        // Electron-based CLIs using electron-log on stdout).
                        loop {
                            line.clear();
                            match reader.read_line(&mut line) {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {}
                            }
                        }
                        break;
                    }
                } else {
                    // The agent wrote something we didn't recognise —
                    // a non-JSON line, a JSON line with an unknown
                    // shape, or stderr mixed into the PTY stream.
                    // Remember the last few so we can surface the
                    // actual reason in the exit-code error.
                    let mut s = trimmed.to_string();
                    if s.len() > TAIL_MAX_LINE {
                        s.truncate(TAIL_MAX_LINE);
                        s.push('…');
                    }
                    if tail.len() == TAIL_CAP {
                        tail.remove(0);
                    }
                    tail.push(s);
                }
            }
        }
    }
    if !terminal {
        match (exit_code_fn(), read_error) {
            // A read *error* with no exit status means the process is (as
            // far as we know) still running and we lost its stream.
            // Fabricating a TurnComplete here made the step executor treat
            // a half-finished agent as done and fail on the missing
            // deliverable. Surface it as an environmental error instead so
            // the driver can retry the step without blaming the
            // implementation.
            (None, Some(err)) => {
                let suffix = if tail.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nLast agent output before the stream broke:\n{}",
                        tail.join("\n")
                    )
                };
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "agent_stream_lost".to_string(),
                    message: format!(
                        "lost the agent's output stream while the agent was still running ({}){}",
                        err, suffix
                    ),
                    recoverable: false,
                });
            }
            // Clean EOF with a zero/unknown exit is the normal "agent
            // finished and closed its stream" ending.
            (Some(0) | None, _) => {
                let _ = tx.blocking_send(AgentEvent::TurnComplete {
                    stop_reason: StopReason::EndOfTurn,
                    usage: None,
                });
            }
            (Some(code), _) => {
                let suffix = if tail.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nLast agent output (unparsed; the JSON parser dropped these — typically stderr or a banner):\n{}",
                        tail.join("\n")
                    )
                };
                let _ = tx.blocking_send(AgentEvent::Error {
                    code: "agent_exit_nonzero".to_string(),
                    message: format!("agent exited with code {}{}", code, suffix),
                    recoverable: false,
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/cli_runtime.rs"]
mod tests;
