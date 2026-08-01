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
use crate::domain::models::{EffortLevel, SessionInfo};
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

/// Translate the session's [`PermissionProfile`](crate::domain::permission::PermissionProfile) into agent-native
/// environment variables (e.g. opencode's `OPENCODE_PERMISSION`). Agents
/// that enforce via CLI flags instead use [`no_permission_env`](crate::ports::agent_runtime::no_permission_env) and read
/// `ctx.permissions` in their [`ArgsBuilder`].
pub type PermEnvBuilder = fn(
    p: &crate::domain::permission::PermissionProfile,
) -> std::collections::HashMap<String, String>;

/// Translate the resolved effort into agent-native environment variables
/// (e.g. claude-code's `CLAUDE_CODE_EFFORT_LEVEL`). Agents that carry effort
/// on argv only — or not at all — use [`no_effort_env`] and read
/// `ctx.effort` in their [`ArgsBuilder`].
pub type EffortEnvBuilder =
    fn(effort: Option<EffortLevel>) -> std::collections::HashMap<String, String>;

/// No agent-native effort env (codex, opencode: argv-only; hermes: no
/// per-invocation effort control at all). The `effort_env` translator for
/// such runtimes — mirrors
/// [`no_permission_env`](crate::ports::agent_runtime::no_permission_env)(crate::ports::agent_runtime::no_permission_env).
pub fn no_effort_env(_effort: Option<EffortLevel>) -> std::collections::HashMap<String, String> {
    std::collections::HashMap::new()
}

/// Inject a runtime's fixed hygiene env ([`UnifiedCliRuntime::static_env`])
/// into a spawn context's env map. Lowest precedence *within demeteo*:
/// `or_insert` (not `extend`) so a caller-provided value in `ctx.env` wins.
/// Like the effort env, these still override the host shell's exports —
/// fleet runs should behave the same on every machine.
pub(crate) fn apply_static_env(
    env: &mut std::collections::HashMap<String, String>,
    static_env: &'static [(&'static str, &'static str)],
) {
    for (k, v) in static_env {
        env.entry((*k).to_string())
            .or_insert_with(|| (*v).to_string());
    }
}

/// Shared runtime for one-shot CLI-based agents (opencode, hermes, claude, etc.)
pub struct UnifiedCliRuntime {
    pub kind_str: &'static str,
    pub binary: &'static str,
    pub install_cmd: &'static str,
    pub parse_event: EventParser,
    pub build_args: ArgsBuilder,
    /// Maps the abstract permission profile to this agent's native env.
    pub perm_env: PermEnvBuilder,
    /// Maps the resolved effort to this agent's native env. Empty for the
    /// three agents that carry effort on argv (or not at all).
    pub effort_env: EffortEnvBuilder,
    /// Human-facing name for pickers/settings (e.g. "Claude Code").
    pub display_label: &'static str,
    /// How this agent enumerates its models, if it can. `None` → the model
    /// picker falls back to a static list.
    pub model_listing: Option<crate::ports::agent_runtime::ModelListing>,
    /// Default model when no override is configured; `None` if not statically
    /// knowable.
    pub default_model: Option<&'static str>,
    /// The effort levels this agent accepts per invocation; `&[]` when it has
    /// no effort control. Surfaced through `capabilities()` to drive the UI
    /// picker.
    pub effort_levels: &'static [EffortLevel],
    /// Fixed environment injected into every spawn of this agent, before the
    /// per-context env (so a caller-provided value wins). For headless CLI
    /// children this is hygiene, not configuration — e.g. claude-code's
    /// `DISABLE_AUTOUPDATER` / `DISABLE_NONESSENTIAL_TRAFFIC`, which cut
    /// spawn latency and background network on fleet machines. `&[]` for
    /// agents with nothing to pin.
    pub static_env: &'static [(&'static str, &'static str)],
}

#[async_trait]
impl AgentRuntime for UnifiedCliRuntime {
    fn kind(&self) -> &'static str {
        self.kind_str
    }

    fn capabilities(&self) -> crate::ports::agent_runtime::AgentCapabilities {
        crate::ports::agent_runtime::AgentCapabilities {
            display_label: self.display_label,
            lists_models: self.model_listing.is_some(),
            model_listing: self.model_listing,
            default_model: self.default_model,
            effort_levels: self.effort_levels,
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
        let effort_env = self.effort_env;
        let static_env = self.static_env;

        Box::pin(async move {
            // Translate the abstract permission profile into this agent's
            // native env. Done here (once, at spawn) so every caller only
            // has to set `ctx.permissions`; arg-based enforcement (e.g.
            // claude-code's --disallowedTools) is layered by build_args
            // reading the same `ctx.permissions`.
            let mut ctx = ctx;
            apply_static_env(&mut ctx.env, static_env);
            ctx.env.extend((perm_env)(&ctx.permissions));
            // Same shape for effort. claude-code needs this even though it
            // also takes `--effort`: `CLAUDE_CODE_EFFORT_LEVEL` outranks the
            // flag and the child inherits the host env (`sanitize_child_env`
            // strips only LD_LIBRARY_PATH / LD_PRELOAD), so a developer who
            // exported the variable would otherwise silently override every
            // Demeteo run.
            ctx.env.extend((effort_env)(ctx.effort));

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
    /// Monotonic high-water mark of the session's request footprint
    /// (input + cache_read + cache_creation + output tokens). Updated
    /// as `Usage` / `UsageDelta` / `TurnComplete { usage }` events are
    /// parsed by `drain_lines`. Read by the driver's context-window watchdog
    /// via [`AgentSession::cumulative_tokens`] — cache reads count,
    /// because they occupy context even though they bill at ~10%.
    /// Zero for a fresh session before the first event arrives.
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
                let msg = spawn_error_message(&e, &self.ctx.binary, binary, text.len());
                let _ = tx.try_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: msg,
                    recoverable: false,
                    usage: None,
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
                // `try_send`, never `blocking_send`: `spawn_remote` runs inline
                // on the async caller's runtime worker (see `prompt`), and
                // `blocking_send` panics ("Cannot block the current thread from
                // within a runtime") there — turning a recoverable, transient
                // SSH spawn failure into a hard panic that kills the driver
                // task and orphans the step at `running` forever. The channel
                // is freshly created with capacity 256, so `try_send` always
                // has room here. Matches `spawn_local`'s error branch.
                let _ = tx.try_send(AgentEvent::Error {
                    code: "spawn_failed".to_string(),
                    message: format!("failed to spawn {} over SSH: {}", self.ctx.binary, e),
                    recoverable: false,
                    usage: None,
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

    fn cwd(&self) -> &str {
        // The directory this session was spawned in. `prompt` always
        // spawns the child with `current_dir(&self.ctx.cwd)` and passes
        // `--dir <cwd>`, but a resumed CLI session (`--session` /
        // `--resume`) writes against the directory it was *originally*
        // created in, so this is the cwd the caller must match to reuse
        // the session safely.
        &self.ctx.cwd
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

/// Probe one parsed stdout line for the agent's session identifier.
///
/// The descriptor carries no key name, so the union of every adapter's
/// spelling is tried against every line. The bare `id` arm is type-guarded:
/// pi names it plainly `id` on its `{"type":"session",…}` header, while
/// messages, tool calls and responses all carry an `id` too — unguarded, that
/// arm would latch onto whichever line arrived first.
pub(crate) fn session_id_from_line(v: &serde_json::Value) -> Option<&str> {
    v.get("sessionID")
        .or_else(|| v.get("session_id"))
        .or_else(|| v.get("conversationID"))
        .or_else(|| v.get("conversation_id"))
        // Codex emits `thread_id` on its first `thread.started` line
        // (`codex exec --json`).
        .or_else(|| v.get("thread_id"))
        .or_else(|| v.get("data").and_then(|d| d.get("conversation_id")))
        .or_else(|| v.get("data").and_then(|d| d.get("session_id")))
        .or_else(|| match v.get("type").and_then(|t| t.as_str()) {
            Some("session") => v.get("id"),
            _ => None,
        })
        .and_then(|s| s.as_str())
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
                                if let Some(sid) = session_id_from_line(&v) {
                                    if let Ok(mut g) = capture.lock() {
                                        *g = Some(sid.to_string());
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(evt) = parse_event(trimmed) {
                    // Track the session's context footprint for the watchdog
                    // (compared against the model's context window at the 80%
                    // threshold). Every adapter reports cache tokens in their
                    // own fields, *separate* from `input_tokens` — on Claude
                    // Code in particular, `usage.input_tokens` is only the
                    // uncached remainder, and on a warm resumed session almost
                    // the entire context is `cache_read_input_tokens`. The
                    // request footprint is therefore
                    // input + cache_read + cache_creation + output; summing
                    // only input+output made the watchdog fire late or never.
                    //
                    // `UsageDelta` is a peer of `Usage` here and is *not*
                    // summed, though the accumulator sums it: this asks
                    // whether one request still fits the context window, so
                    // the largest single footprint answers it and the billed
                    // total does not.
                    if let Some(ref cumulative) = cumulative_tokens {
                        let footprint = |u: &crate::domain::agent_event::Usage| {
                            u.input_tokens
                                + u.cache_read_input_tokens
                                + u.cache_creation_input_tokens
                                + u.output_tokens
                        };
                        let delta = match &evt {
                            AgentEvent::Usage(u) | AgentEvent::UsageDelta(u) => footprint(u),
                            AgentEvent::TurnComplete { usage: Some(u), .. } => footprint(u),
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
                    let is_terminal = evt.ends_turn();
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
                    usage: None,
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
                    usage: None,
                });
            }
        }
    }
}

/// How a failed local spawn must be described to the user. Pure over the error,
/// so the whole policy is decidable in a unit test — `spawn_local` itself is
/// only reachable by standing up a session and an `AgentContext` it never reads.
///
/// Three shapes, and they are three *different* problems:
///
/// - `NotFound` — the agent isn't installed. The user's machine, the user's fix.
/// - `ArgumentListTooLong` (`E2BIG`) — **ours**. The prompt we built exceeded the
///   ceiling the OS puts on a single command-line argument, so `execve` refused
///   and no agent process ever existed. Raw, this surfaces as `os error 7` under
///   the `[environment — not an implementation failure]` banner, which sends the
///   user auditing a machine that is fine; it cost one observed run its whole
///   pipeline after `s-implement` had already spent 3.8 M tokens. Name the size,
///   the ceiling, and whose fault it is. See `domain::prompt_budget`.
/// - anything else — pass the OS's own words through; we have nothing to add.
fn spawn_error_message(
    e: &std::io::Error,
    agent_binary: &str,
    resolved_binary: &str,
    prompt_bytes: usize,
) -> String {
    match e.kind() {
        std::io::ErrorKind::NotFound => format!("binary not found at '{}'", resolved_binary),
        std::io::ErrorKind::ArgumentListTooLong => {
            crate::domain::prompt_budget::argv_too_long_message(agent_binary, prompt_bytes)
        }
        _ => format!(
            "failed to spawn {} ({}): {}",
            agent_binary, resolved_binary, e
        ),
    }
}

#[cfg(test)]
#[path = "../../../tests/infrastructure/agent/cli_runtime.rs"]
mod tests;
