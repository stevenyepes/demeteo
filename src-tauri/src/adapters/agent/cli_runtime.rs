use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader, Write};
use std::sync::Mutex;

use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;

use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::domain::models::SessionInfo;
use crate::ports::agent_runtime::{AgentContext, AgentRuntime, AgentSession, AgentStartError};

/// Parse a single JSON-lines event from a CLI agent's stdout.
/// Returns `None` if the line can't be parsed or is not a recognized event.
pub type EventParser = fn(line: &str) -> Option<AgentEvent>;

/// Shared runtime for non-ACP CLI agents (Claude Code, Antigravity, etc.)
/// that stream JSON-lines on stdout.
///
/// Each CLI agent registers a `parse_event_line` function pointer so this
/// shared runtime stays transport-neutral; only the parsing logic differs.
pub struct CliAgentRuntime {
    pub kind_str: &'static str,
    pub binary: &'static str,
    pub extra_args: &'static [&'static str],
    pub install_cmd: &'static str,
    pub parse_event: EventParser,
}

impl AgentRuntime for CliAgentRuntime {
    fn kind(&self) -> &'static str {
        self.kind_str
    }

    fn is_available(&self, exec: &dyn crate::ports::execution::ExecutionPort, machine_id: &str) -> bool {
        if machine_id == "local" || machine_id.is_empty() {
            // Quick which/command -v check on the local PATH.
            Command::new("which")
                .arg(self.binary)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            exec.run_command(machine_id, &format!("command -v {} >/dev/null 2>&1 && echo ok", self.binary))
                .map(|out| out.trim() == "ok")
                .unwrap_or(false)
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
        let binary = self.binary;
        let extra_args: Vec<&'static str> = self.extra_args.to_vec();
        let parse_event = self.parse_event;
        let kind = self.kind_str;

        Box::pin(async move {
            // Build the command. The full prompt is delivered via stdin after spawn.
            let mut child = Command::new(binary)
                .args(&extra_args)
                .current_dir(&ctx.cwd)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| AgentStartError::SpawnFailed(format!("{}: {}", kind, e)))?;

            let stdin = child.stdin.take().expect("stdin was piped");
            let stdout = child.stdout.take().expect("stdout was piped");

            let session = CliAgentSession {
                session_id: format!("{}-{}", kind, ctx.thread_id),
                stdin: Mutex::new(Some(stdin)),
                stdout: Mutex::new(Some(BufReader::new(stdout))),
                parse_event,
            };

            Ok(Arc::new(session) as Arc<dyn AgentSession>)
        })
    }
}

pub struct CliAgentSession {
    session_id: String,
    stdin: Mutex<Option<std::process::ChildStdin>>,
    stdout: Mutex<Option<BufReader<std::process::ChildStdout>>>,
    parse_event: EventParser,
}

impl AgentSession for CliAgentSession {
    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
        // Write the prompt to stdin, then drain stdout line-by-line.
        if let Ok(mut guard) = self.stdin.lock() {
            if let Some(ref mut stdin) = *guard {
                let _ = stdin.write_all(text.as_bytes());
                let _ = stdin.write_all(b"\n");
                let _ = stdin.flush();
            }
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(256);
        let parse_event = self.parse_event;

        // Move stdout reader ownership to the draining thread.
        let reader_opt = {
            if let Ok(mut guard) = self.stdout.lock() {
                guard.take()
            } else {
                None
            }
        };

        std::thread::spawn(move || {
            if let Some(mut reader) = reader_opt {
                let mut line = String::new();
                loop {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let trimmed = line.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if let Some(evt) = parse_event(trimmed) {
                                let is_terminal = matches!(evt, AgentEvent::TurnComplete { .. } | AgentEvent::Error { .. });
                                let _ = tx.blocking_send(evt);
                                if is_terminal {
                                    break;
                                }
                            }
                        }
                    }
                }
                // Emit TurnComplete if process closed stdout without it
                let _ = tx.blocking_send(AgentEvent::TurnComplete { stop_reason: StopReason::EndOfTurn });
            }
        });

        Box::pin(ReceiverStream::new(rx))
    }

    fn cancel(&self) -> Result<(), String> {
        // Drop stdin to signal EOF to the child process.
        if let Ok(mut guard) = self.stdin.lock() {
            *guard = None;
        }
        Ok(())
    }

    fn set_mode(&self, _mode_id: &str) -> Result<(), String> {
        // CLI agents don't support ACP modes. No-op with a warning.
        Ok(())
    }

    fn set_config_option(&self, _config_id: &str, _value: &str) -> Result<(), String> {
        Ok(())
    }

    fn session_info(&self) -> SessionInfo {
        SessionInfo::default()
    }
}
