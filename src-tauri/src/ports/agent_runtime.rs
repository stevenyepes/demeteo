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
}

/// Default permission policy injected into every opencode agent process as
/// the `OPENCODE_PERMISSION` env var. The binary enforces this at spawn time;
/// `external_directory: "deny"` scopes the agent to its worktree.
const DEFAULT_OPENCODE_PERMISSION: &str = r#"{"edit":"allow","read":"allow","bash":"allow","webfetch":"deny","websearch":"deny","external_directory":"deny","doom_loop":"allow"}"#;

/// Standard environment variables injected into every agent process.
pub fn agent_base_env() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "OPENCODE_PERMISSION".to_string(),
        DEFAULT_OPENCODE_PERMISSION.to_string(),
    );
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
