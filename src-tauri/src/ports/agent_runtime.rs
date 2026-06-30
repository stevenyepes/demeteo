use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_stream::Stream;

use crate::domain::agent_event::AgentEvent;
use crate::domain::models::SessionInfo;
use crate::domain::permission::PermissionProfile;
use crate::ports::agent_execution::AgentExecutionPort;

pub type AgentStartFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + 'a>>;

#[derive(Clone)]
pub struct AgentContext {
    pub thread_id: String,
    pub machine_id: String,
    pub binary: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
    /// Optional model identifier to pass as initial configOption
    /// in the `session/new` request (e.g. "deepseek", "gpt-4o").
    pub model: Option<String>,
    /// Optional step title, passed as `--title <value>` for CLI agents
    /// that support named sessions (opencode, hermes).
    pub title: Option<String>,
    /// The policy-enforced execution port. Used by the tool bridge
    /// inside the runtime to dispatch agent-originated file/terminal
    /// requests through the existing policy + scope-fence machinery.
    pub agent_exec: Arc<dyn AgentExecutionPort>,
    /// The execution port for spawning processes locally or remotely.
    pub exec: Arc<dyn crate::ports::execution::ExecutionPort>,
    /// The agent-agnostic permission posture for this session. The
    /// concrete runtime translates it into native enforcement at spawn
    /// (opencode → `OPENCODE_PERMISSION` env; claude-code →
    /// `--disallowedTools`). Defaults to `all_allow` for interactive /
    /// probe sessions that aren't capability-scoped pipeline steps.
    pub permissions: PermissionProfile,
    /// Opt the claude-code session into `--bare` and
    /// `--exclude-dynamic-system-prompt-sections` so the vendor
    /// system prompt is byte-identical across worktrees (better prompt
    /// cache reuse). Default `false`; the orchestrator passes `true`
    /// for capability-scoped pipeline steps and leaves it off for the
    /// interactive AgentTerminalDrawer so CLAUDE.md / hooks /
    /// skills still auto-load.
    pub bare_mode: bool,
}

/// Render a [`PermissionProfile`] to the `OPENCODE_PERMISSION` JSON string.
///
/// The policy is *complete* (every gated tool has an explicit value) and
/// only ever uses `allow` / `deny` — never `ask` — so opencode runs fully
/// non-interactively with no permission prompts. `external_directory` is
/// always `deny` (scopes the agent to its worktree); `read` is always
/// `allow` (file reads, grep/glob/list are separate read tools, *not* the
/// shell, so denying `bash` never blocks codebase inspection).
pub fn opencode_permission_json(p: &PermissionProfile) -> String {
    format!(
        r#"{{"edit":"{edit}","read":"{read}","bash":"{bash}","webfetch":"{web}","websearch":"{web}","external_directory":"deny","doom_loop":"allow"}}"#,
        edit = p.write_fs.opencode_str(),
        read = p.read_fs.opencode_str(),
        bash = p.execute.opencode_str(),
        web = p.network.opencode_str(),
    )
}

/// The `OPENCODE_PERMISSION` env entry for a profile. Used as the
/// `perm_env` translator for opencode-family runtimes.
pub fn opencode_permission_env(p: &PermissionProfile) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "OPENCODE_PERMISSION".to_string(),
        opencode_permission_json(p),
    );
    env
}

/// No agent-native permission env (e.g. claude-code, which enforces via
/// CLI flags instead). The `perm_env` translator for such runtimes.
pub fn no_permission_env(_p: &PermissionProfile) -> HashMap<String, String> {
    HashMap::new()
}

/// Standard, permission-independent environment variables injected into
/// every agent process. Permission policy is applied separately by the
/// runtime from [`AgentContext::permissions`], so this no longer carries
/// `OPENCODE_PERMISSION`.
///
/// USER is explicitly forwarded because some CLIs (notably the native
/// Claude Code install) use it to locate credentials. When the Tauri GUI
/// app launches without a full login-session environment (e.g. from
/// Finder/Dock on macOS), USER may be absent from the inherited env;
/// deriving it here from the parent process ensures child agents always
/// have it.
///
/// `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` are intentionally **not**
/// injected here. Because we no longer pass `--bare` (which would set
/// `CLAUDE_CODE_SIMPLE=1` and disable keychain/OAuth reads), Claude Code
/// resolves and refreshes its own credential natively from the keychain
/// (macOS) or `~/.claude/.credentials.json` (all OSes). Demeteo handles
/// no Anthropic credentials at all. A user who exports
/// `ANTHROPIC_API_KEY=...` in their shell is still inherited and honored.
pub fn agent_base_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    for key in ["USER", "LOGNAME", "HOME", "SHELL", "TMPDIR"] {
        if let Ok(val) = std::env::var(key) {
            env.insert(key.to_string(), val);
        }
    }
    env
}

#[derive(Debug, Error)]
pub enum AgentStartError {
    #[error("agent binary not found: {0}")]
    NotFound(String),

    #[error("user declined to install {agent}: install_command was: `{command}`")]
    InstallDeclined { agent: String, command: String },

    #[error("install script failed: {0}")]
    InstallFailed(String),

    #[error("agent failed to start: {0}")]
    SpawnFailed(String),
}

/// Transport-neutral runtime for a single agent. The runtime takes a binary
/// and a config and owns the lifecycle of one agent session: spawning,
/// initialization, prompt streaming, cancel, and clean teardown.
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Stable identifier; matches `AgentConfig.kind`.
    fn kind(&self) -> &'static str;

    /// The actual executable name on disk. Defaults to [`Self::kind`], which
    /// is correct when the kind matches the binary (opencode, hermes, …).
    /// Runtimes whose kind is a hyphenated label that doesn't exist as a
    /// binary on `$PATH` (e.g. `claude-code` kind → `claude` binary) must
    /// override this so the executor spawns the right process.
    fn binary(&self) -> &'static str {
        self.kind()
    }

    /// Check if the binary is reachable on the target host (which / command
    /// -v). The result is cached per `(machine_id, kind)` by the registry for
    /// the duration of the app session.
    async fn is_available(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> bool;

    /// The official install command, shown verbatim in the consent prompt.
    fn install_command(&self) -> &'static str;

    /// The model this runtime selects when no explicit override is configured.
    /// Used to seed `UsageAccumulator` so the pricing-table fallback can
    /// compute `cost_usd` even when the agent's wire format omits it.
    ///
    /// Returns `None` when the runtime selects the model dynamically (e.g.
    /// from an environment variable or project config) and the default is not
    /// statically knowable.
    fn default_model(&self) -> Option<String> {
        None
    }

    /// Spawn the agent and return a session handle. The session is fully
    /// initialized (capability negotiation, session/new, etc.) before this
    /// returns. Specific protocol-level work lives in concrete adapters.
    ///
    /// Async because the runtime may need to do network I/O during
    /// `initialize` / `session/new`; the return is a boxed future so
    /// the trait stays dyn-safe.
    fn start(&self, ctx: AgentContext) -> AgentStartFuture<'_>;
}

pub trait AgentSession: Send + Sync {
    /// The runtime's own session id; never escapes the backend.
    fn session_id(&self) -> &str;

    /// Submit a directive. The returned stream yields `AgentEvent`s until
    /// `TurnComplete` (or terminal `Error`) is emitted, at which point the
    /// stream closes.
    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

    /// Cancel the in-flight turn. Idempotent: no-op if turn is already done.
    fn cancel(&self) -> Result<(), String>;

    /// Switch the agent's operating mode (e.g. "plan", "build", "ask", "code").
    /// Sends a `session/set_mode` ACP call. Returns an error if the agent
    /// does not support the requested mode.
    fn set_mode(&self, mode_id: &str) -> Result<(), String>;

    /// Change a session configuration option (e.g. model, reasoning level).
    /// Sends a `session/set_config_option` ACP call with the given config id
    /// and value. The agent must have advertised the option during setup.
    fn set_config_option(&self, config_id: &str, value: &str) -> Result<(), String>;

    /// Return the session info captured at startup (modes, config options, etc.)
    /// so the frontend can display available choices to the user.
    fn session_info(&self) -> SessionInfo;

    /// Forcibly tear down the session's transport. Used by one-shot
    /// callers (e.g. the model probe in `get_agent_models`) that need
    /// to abort the session without sending a cooperative
    /// `session/cancel` first. Default is a no-op for sessions that
    /// don't hold a transport handle (CLI agents, noop).
    fn kill(&self) -> Result<(), String> {
        Ok(())
    }
    /// Return a handle that signals stderr activity from the underlying
    /// agent process. The step executor uses this to differentiate "agent
    /// is working (API call, model inference)" from "agent is blocked
    /// (no stdout + no stderr)". Sessions that don't track stderr return
    /// `None` — the executor falls back to the standard timeout.
    fn stderr_heartbeat(&self) -> Option<StderrHeartbeat> {
        None
    }

    /// Whether the underlying agent process / SSH channel is still
    /// alive. Used by the driver's context-window watchdog: when a
    /// session dies between steps (network blip, crash), the next
    /// `spawn_agent_session` should fall back to `registry.kill` +
    /// fresh spawn instead of trying to `--continue` against a dead
    /// id. Default `true` so no-op runtimes (NoopRuntime, future
    /// in-process adapters) participate without ceremony.
    fn is_alive(&self) -> bool {
        true
    }

    /// Cumulative input+output tokens billed against this session's
    /// underlying agent process. Used by the watchdog to compare
    /// against the model's context-window budget (see
    /// `PricingTable::context_window`). Default `0` for runtimes that
    /// can't track this in process (NoopRuntime); the watchdog treats
    /// that as "no data, skip check."
    fn cumulative_tokens(&self) -> u64 {
        0
    }
}

/// Cheaply-cloneable handle that tracks how recently the agent's stderr
/// produced output. The stderr drain thread calls [`beat`] on every
/// line; the step executor polls [`last_activity_ago_ms`] to decide
/// whether the process is truly stuck.
#[derive(Clone)]
pub struct StderrHeartbeat {
    last_ts: Arc<AtomicU64>,
}

impl Default for StderrHeartbeat {
    fn default() -> Self {
        Self::new()
    }
}

impl StderrHeartbeat {
    pub fn new() -> Self {
        Self {
            last_ts: Arc::new(AtomicU64::new(Self::now_ms())),
        }
    }

    /// Call from the stderr drain thread every time a complete line is
    /// received from the agent's stderr pipe.
    pub fn beat(&self) {
        self.last_ts.store(Self::now_ms(), Ordering::Relaxed);
    }

    /// Milliseconds since the last call to [`beat`] (or since construction
    /// if `beat` was never called).
    pub fn last_activity_ago_ms(&self) -> u64 {
        Self::now_ms().saturating_sub(self.last_ts.load(Ordering::Relaxed))
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }
}

/// Stub for SerializedAgentConfig — this is here for future use by the
/// settings UI (Phase 7d) where per-agent fields (model, work_dir, env)
/// become configurable. Currently a placeholder so the trait surface
/// compiles end-to-end.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SerializedAgentConfig {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub work_dir: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}
