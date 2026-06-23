use std::future::Future;
use std::io::{BufRead, BufReader, Read, Write};
use std::pin::Pin;
use std::process::{Command, Stdio};
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
pub type ArgsBuilder = fn(ctx: &AgentContext, captured_session_id: Option<&str>) -> Vec<String>;

/// Shared runtime for one-shot CLI-based agents (opencode, hermes, claude, agy, etc.)
pub struct UnifiedCliRuntime {
    pub kind_str: &'static str,
    pub binary: &'static str,
    pub install_cmd: &'static str,
    pub parse_event: EventParser,
    pub build_args: ArgsBuilder,
}

#[async_trait]
impl AgentRuntime for UnifiedCliRuntime {
    fn kind(&self) -> &'static str {
        self.kind_str
    }

    async fn is_available(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> bool {
        if machine_id == "local" || machine_id.is_empty() {
            super::is_binary_on_local_path(self.binary)
        } else {
            let res = exec
                .run_command(
                    machine_id,
                    &format!("command -v {} >/dev/null 2>&1 && echo ok", self.binary),
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

        Box::pin(async move {
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
}

impl UnifiedCliSession {
    fn build_command(&self) -> Command {
        let binary = self.resolved_binary.as_deref().unwrap_or(&self.ctx.binary);
        let mut cmd = Command::new(binary);
        let captured = {
            if let Ok(guard) = self.captured_session_id.lock() {
                guard.clone()
            } else {
                None
            }
        };
        let args = (self.build_args)(&self.ctx, captured.as_deref());
        cmd.args(&args);
        cmd.current_dir(&self.ctx.cwd);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        for (k, v) in &self.ctx.env {
            cmd.env(k, v);
        }
        cmd
    }

    fn spawn_local(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
    ) {
        let mut cmd = self.build_command();
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

        if let Some(mut stdin) = child.stdin.take() {
            let text_owned = text.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(text_owned.as_bytes());
                let _ = stdin.write_all(b"\n");
                let _ = stdin.flush();
            });
        }

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

        std::thread::spawn(move || {
            drain_lines(
                BufReader::new(stdout),
                parse_event,
                exit_code_fn,
                tx,
                Some(session_capture),
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
        let args = (self.build_args)(&self.ctx, captured.as_deref());
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
                .and_then(|h| h.try_wait().ok().flatten())
        };

        let reader = HandleReader {
            handle: handle.clone(),
        };
        let session_capture = self.captured_session_id.clone();
        std::thread::spawn(move || {
            drain_lines(reader, parse_event, exit_code_fn, tx, Some(session_capture));
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
        let h = self.handle.lock().expect("HandleReader mutex poisoned");
        h.try_read(buf)
    }
}

fn drain_lines<R, F>(
    reader: R,
    parse_event: EventParser,
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
                    message: format!("agent exited with code {}", code),
                    recoverable: false,
                });
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/cli_runtime.rs"]
mod tests;
