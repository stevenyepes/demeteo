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

use super::trace::TurnTrace;
use crate::domain::agent_event::{AgentEvent, StopReason};
use crate::domain::models::{Availability, EffortLevel, SessionInfo};
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
    /// What [`AgentContext::bare_mode`] does to this harness's own
    /// personalization. Declared beside the argv builder because
    /// [`build_args`](Self::build_args) is the only evidence for it.
    pub personalization: crate::ports::agent_runtime::PersonalizationSupport,
    /// Fixed environment injected into every spawn of this agent, before the
    /// per-context env (so a caller-provided value wins). For headless CLI
    /// children this is hygiene, not configuration — e.g. claude-code's
    /// `DISABLE_AUTOUPDATER` / `DISABLE_NONESSENTIAL_TRAFFIC`, which cut
    /// spawn latency and background network on fleet machines. `&[]` for
    /// agents with nothing to pin.
    pub static_env: &'static [(&'static str, &'static str)],
    /// The interpreter this CLI runs agent-authored commands under on Windows.
    /// Declared beside the argv builder because that is the code that knows
    /// how this harness invokes anything at all.
    pub windows_agent_shell: crate::domain::models::WindowsAgentShell,
    /// Env that makes [`windows_agent_shell`](Self::windows_agent_shell) true
    /// instead of probable, applied only where that declaration is load-bearing
    /// — see [`pinned_shell_env`](crate::domain::agent_env::pinned_shell_env),
    /// which owns the narrowing. `&[]` for a harness whose Windows shell is not
    /// switchable, or whose declaration is `Unknown` and has nothing to pin.
    pub windows_shell_env: &'static [(&'static str, &'static str)],
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
            personalization: self.personalization,
            windows_agent_shell: self.windows_agent_shell,
        }
    }

    fn binary(&self) -> &'static str {
        self.binary
    }

    async fn availability(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> Availability {
        if machine_id == "local" || machine_id.is_empty() {
            // A `PATH` scan on this host either finds the binary or does not;
            // there is no third answer to report.
            if super::is_binary_on_local_path(self.binary) {
                Availability::Installed
            } else {
                Availability::Missing
            }
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
            Availability::from_probe(res, "ok")
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
        let windows_shell_env = self.windows_shell_env;

        Box::pin(async move {
            // Translate the abstract permission profile into this agent's
            // native env. Done here (once, at spawn) so every caller only
            // has to set `ctx.permissions`; arg-based enforcement (e.g.
            // claude-code's --disallowedTools) is layered by build_args
            // reading the same `ctx.permissions`.
            let mut ctx = ctx;
            apply_static_env(&mut ctx.env, static_env);
            apply_static_env(
                &mut ctx.env,
                crate::domain::agent_env::pinned_shell_env(ctx.platform, windows_shell_env),
            );
            ctx.env.extend((perm_env)(&ctx.permissions));
            // Same shape for effort. claude-code needs this even though it
            // also takes `--effort`: `CLAUDE_CODE_EFFORT_LEVEL` outranks the
            // flag and the child inherits the host env (`sanitize_child_env`
            // strips only LD_LIBRARY_PATH / LD_PRELOAD), so a developer who
            // exported the variable would otherwise silently override every
            // Demeteo run.
            ctx.env.extend((effort_env)(ctx.effort));

            let local_launch = if ctx.machine_id.is_empty() || ctx.machine_id == "local" {
                super::resolve_local_binary_path(&ctx.binary)
            } else {
                None
            };
            let session = UnifiedCliSession {
                session_id: format!("{}-{}", kind, ctx.thread_id),
                local_launch,
                ctx,
                parse_event,
                build_args,
                live_local: Mutex::new(None),
                live_remote: Mutex::new(None),
                captured_session_id: Arc::new(Mutex::new(None)),
                stderr_hb: StderrHeartbeat::new(),
                cumulative_tokens: Arc::new(AtomicU64::new(0)),
                turn_seq: AtomicU64::new(0),
            };
            Ok(Arc::new(session) as Arc<dyn AgentSession>)
        })
    }
}

#[allow(clippy::type_complexity)]
pub struct UnifiedCliSession {
    session_id: String,
    local_launch: Option<super::LocalAgentLaunch>,
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
    /// Turns issued against this session so far. Its only consumer is the
    /// raw-capture file name, which needs the turns of one session to be
    /// distinguishable and ordered; a wall-clock stamp would give neither
    /// when two turns land in the same millisecond.
    turn_seq: AtomicU64,
}

/// Where the drain thread reports, besides the event channel.
struct DrainSinks {
    session_capture: Option<Arc<Mutex<Option<String>>>>,
    cumulative_tokens: Option<Arc<AtomicU64>>,
    /// The binary named in the `agent_no_output` message, which is the one
    /// ending whose remedy is to run that binary by hand.
    agent: String,
    /// The turn's raw capture, `None` unless a developer asked for one
    /// (`adapters::agent::trace`).
    trace: Option<TurnTrace>,
}

impl UnifiedCliSession {
    fn build_command(&self, prompt: &str) -> Command {
        let binary = self
            .local_launch
            .as_ref()
            .map(|launch| launch.executable.as_str())
            .unwrap_or(&self.ctx.binary);
        let mut cmd = Command::new(binary);
        let captured = {
            if let Ok(guard) = self.captured_session_id.lock() {
                guard.clone()
            } else {
                None
            }
        };
        let args = (self.build_args)(&self.ctx, captured.as_deref(), prompt);
        if let Some(launch) = &self.local_launch {
            cmd.args(&launch.prefix_args);
        }
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
        // No console for a headless turn. Without it a packaged Windows build
        // — `windows_subsystem = "windows"`, so the app owns no console —
        // flashes one per agent invocation, and the child inherits a console
        // it could read stdin from. `Stdio::null()` above already forecloses
        // the read; this forecloses the window.
        crate::shared::proc::harden_child_spawn(&mut cmd);
        cmd
    }

    fn drain_sinks(&self, trace: Option<TurnTrace>) -> DrainSinks {
        DrainSinks {
            session_capture: Some(self.captured_session_id.clone()),
            cumulative_tokens: Some(self.cumulative_tokens.clone()),
            agent: self.ctx.binary.clone(),
            trace,
        }
    }

    fn spawn_local(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        trace: Option<TurnTrace>,
    ) {
        let mut cmd = self.build_command(text);
        let binary = self
            .local_launch
            .as_ref()
            .map(|launch| launch.executable.as_str())
            .unwrap_or(&self.ctx.binary);

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

        let sinks = self.drain_sinks(trace);

        reap_when_abandoned(child, tx.downgrade());

        std::thread::spawn(move || {
            drain_lines(BufReader::new(stdout), parse_event, exit_code_fn, tx, sinks);
        });
    }

    fn spawn_remote(
        &self,
        text: &str,
        parse_event: EventParser,
        tx: tokio::sync::mpsc::Sender<AgentEvent>,
        trace: Option<TurnTrace>,
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
        let sinks = self.drain_sinks(trace);
        std::thread::spawn(move || {
            drain_lines(reader, parse_event, exit_code_fn, tx, sinks);
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

        // Opened here, not per transport: a capture that only local turns
        // produced would answer the diagnostic question for one half of the
        // ExecutionPort contract and quietly not for the other.
        let turn = self.turn_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let trace = TurnTrace::open(&self.session_id, turn);

        if is_local {
            self.spawn_local(text, parse_event, tx, trace);
        } else {
            self.spawn_remote(text, parse_event, tx, trace);
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

/// Absolute ceiling on one spawned agent process, owned by the spawner.
///
/// [`stream_agent_turn`](super::event_stream::turn::stream_agent_turn) bounds
/// the *turn*, not the process: on its silence or wall-clock deadline it
/// returns a verdict and drops the stream, and nothing between there and the
/// next `prompt()` stops the child — after an abandoned turn there is no next
/// `prompt()` at all. On Unix the leftover is an idle process. On Windows it is
/// a set of open handles under the worktree, and directory removal fails
/// outright against one, so a single hung turn leaves a worktree that can never
/// be torn down.
///
/// Above the largest `wall_cap_s`
/// [`AgentTimeouts::validated`](crate::domain::models::AgentTimeouts::validated)
/// accepts, so it can never pre-empt a turn a user configured. The abandonment
/// check is what makes the turn's own deadline bind the process; this is the
/// floor under a consumer that never went away either.
const PROCESS_CEILING: std::time::Duration = std::time::Duration::from_secs(4 * 3600 + 900);

/// How long a child may outlive the consumer of its output before it is killed.
///
/// Not zero: a CLI agent writes its own session store on the way out, and that
/// store is exactly what the next turn's `--resume` reads. It is also longer
/// than the gap this replaces in the common path — today a finished child lives
/// until the *next* `prompt()`, which the driver usually issues within seconds.
const ABANDONED_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

/// Kill the child once nothing is listening, or at [`PROCESS_CEILING`],
/// whichever comes first.
///
/// The consumer is held weakly on purpose: a strong `Sender` clone would keep
/// the channel open past `drain_lines`, and callers that read the stream to
/// completion would block on this thread's timer instead of the agent.
///
/// Local children only. The remote handle's mutex is held across a ~10s
/// blocking read (see [`HandleReader`]), so polling it from a second thread
/// would interleave with the read loop the SSH keepalive depends on — and the
/// motivation above is a Windows filesystem fact about the *host*, which is
/// never the machine a remote agent leaves a process on.
fn reap_when_abandoned(
    child: Arc<Mutex<std::process::Child>>,
    consumer: tokio::sync::mpsc::WeakSender<AgentEvent>,
) {
    const TICK: std::time::Duration = std::time::Duration::from_millis(200);
    std::thread::spawn(move || {
        let ceiling = std::time::Instant::now() + PROCESS_CEILING;
        let mut abandoned_since: Option<std::time::Instant> = None;
        loop {
            std::thread::sleep(TICK);
            let Ok(mut child) = child.lock() else { return };
            match child.try_wait() {
                Ok(Some(_)) | Err(_) => return,
                Ok(None) => {}
            }
            let now = std::time::Instant::now();
            if consumer.upgrade().is_some_and(|tx| !tx.is_closed()) {
                abandoned_since = None;
            } else if abandoned_since.is_none() {
                abandoned_since = Some(now);
            }
            let abandoned_long_enough =
                abandoned_since.is_some_and(|since| now.duration_since(since) >= ABANDONED_GRACE);
            if now < ceiling && !abandoned_long_enough {
                continue;
            }
            let _ = child.kill();
            let _ = child.wait();
            return;
        }
    });
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

/// How a one-shot agent process ended, for the cases where it emitted no
/// terminal event of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnEnding {
    /// The process wrote, then closed its stream and exited cleanly.
    Complete,
    /// The stream broke while the process was, as far as we know, still alive.
    StreamLost,
    /// A non-zero exit — the one ending that is the process's own verdict.
    NonZeroExit(i32),
    /// A clean exit that produced nothing at all.
    NoOutput,
}

impl TurnEnding {
    /// The [`AgentEvent::Error`] code this ending is reported under, or `None`
    /// for the one ending that is a turn.
    pub(crate) fn error_code(self) -> Option<&'static str> {
        match self {
            TurnEnding::Complete => None,
            TurnEnding::StreamLost => Some("agent_stream_lost"),
            TurnEnding::NonZeroExit(_) => Some("agent_exit_nonzero"),
            TurnEnding::NoOutput => Some("agent_no_output"),
        }
    }
}

/// Whether an [`AgentEvent::Error`] names a *process* that never reached a
/// verdict, as opposed to a verdict the agent itself reported.
///
/// The distinction the turn loop routes on, and the same one
/// [`classify_exec_failure`](crate::domain::harness_failure::classify_exec_failure)
/// draws for a harness command: a process that could not run, or ran and told
/// us nothing, tested nothing. Feeding that into a rework cycle spends the
/// retry budget re-implementing code no edit can make green. A `cli_error` —
/// the agent's own report of what went wrong with the work — is the only one
/// worth handing back as feedback.
pub(crate) fn is_process_level_error(code: &str) -> bool {
    matches!(
        code,
        "spawn_failed" | "agent_stream_lost" | "agent_exit_nonzero" | "agent_no_output"
    )
}

/// Read the end of a one-shot agent's stdout as exactly one of four endings.
///
/// Pure over what [`drain_lines`] observed, because the four are
/// indistinguishable at the call site and only one of them is a turn.
///
/// [`TurnEnding::NoOutput`] is the one that is not obvious. A process that
/// exits 0 having written nothing did not run an empty turn — it did not run.
/// Read as `Complete` it becomes a green turn that merely produced no
/// deliverable, so the step's verdict is fabricated rather than measured, which
/// in a human-approval-gated orchestrator is the worst outcome available. It is
/// the documented Windows signature of several of these CLIs, and it is also
/// what a `.cmd` shim does when the interpreter behind it is gone.
///
/// A verdict is deliberately *not* what it produces: nothing ran, so nothing
/// was tested. `docs/EXECUTION_PARITY.md` D3 makes the same call for a
/// transport error and a timeout.
pub(crate) fn classify_turn_ending(
    exit_code: Option<i32>,
    stream_broke: bool,
    wrote_output: bool,
) -> TurnEnding {
    match exit_code {
        // A read error with no exit status means the process is still running
        // and we lost its stream. Fabricating a completion here made the step
        // executor treat a half-finished agent as done and then fail on the
        // missing deliverable.
        None if stream_broke => TurnEnding::StreamLost,
        Some(code) if code != 0 => TurnEnding::NonZeroExit(code),
        _ if !wrote_output => TurnEnding::NoOutput,
        _ => TurnEnding::Complete,
    }
}

fn drain_lines<R, F>(
    reader: R,
    parse_event: EventParser,
    exit_code_fn: F,
    tx: tokio::sync::mpsc::Sender<AgentEvent>,
    sinks: DrainSinks,
) where
    R: Read,
    F: FnOnce() -> Option<i32>,
{
    let DrainSinks {
        session_capture,
        cumulative_tokens,
        agent,
        mut trace,
    } = sinks;
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
    // Whether the process said anything at all, parseable or not — the input
    // `classify_turn_ending` needs, and deliberately not "did any of it parse".
    let mut wrote_output = false;
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
                wrote_output = true;
                // Before the parser, so the capture holds what the agent said
                // rather than what this runtime recognised — the lines it does
                // not recognise are the ones worth having.
                if let Some(trace) = trace.as_mut() {
                    trace.record(trimmed);
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
        let ending = classify_turn_ending(exit_code_fn(), read_error.is_some(), wrote_output);
        let message = match ending {
            TurnEnding::Complete => None,
            TurnEnding::StreamLost => {
                let err = read_error.map(|e| e.to_string()).unwrap_or_default();
                let suffix = if tail.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nLast agent output before the stream broke:\n{}",
                        tail.join("\n")
                    )
                };
                Some(format!(
                    "lost the agent's output stream while the agent was still running ({}){}",
                    err, suffix
                ))
            }
            TurnEnding::NonZeroExit(code) => {
                let suffix = if tail.is_empty() {
                    String::new()
                } else {
                    format!(
                        "\n\nLast agent output (unparsed; the JSON parser dropped these — typically stderr or a banner):\n{}",
                        tail.join("\n")
                    )
                };
                Some(format!("agent exited with code {}{}", code, suffix))
            }
            TurnEnding::NoOutput => Some(format!(
                "{} exited 0 without writing a single line, so no turn ran. \
                 Nothing was attempted and nothing was tested — check that the \
                 binary on this machine is the agent and that it can start \
                 (run it by hand in the worktree).",
                agent
            )),
        };
        let event = match (ending.error_code(), message) {
            (Some(code), Some(message)) => AgentEvent::Error {
                code: code.to_string(),
                message,
                recoverable: false,
                usage: None,
            },
            _ => AgentEvent::TurnComplete {
                stop_reason: StopReason::EndOfTurn,
                usage: None,
            },
        };
        let _ = tx.blocking_send(event);
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
