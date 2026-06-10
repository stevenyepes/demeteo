use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_stream::Stream;

use crate::domain::agent_event::AgentEvent;
use crate::domain::models::SessionInfo;
use crate::ports::agent_execution::AgentExecutionPort;

#[derive(Clone)]
pub struct AgentContext {
    pub thread_id: String,
    pub machine_id: String,
    pub binary: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
    /// The policy-enforced execution port. Used by the tool bridge
    /// inside the runtime to dispatch agent-originated file/terminal
    /// requests through the existing policy + scope-fence machinery.
    pub agent_exec: Arc<dyn AgentExecutionPort>,
    /// The execution port for spawning processes locally or remotely.
    pub exec: Arc<dyn crate::ports::execution::ExecutionPort>,
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
pub trait AgentRuntime: Send + Sync {
    /// Stable identifier; matches `AgentConfig.kind`.
    fn kind(&self) -> &'static str;

    /// Check if the binary is reachable on the target host (which / command
    /// -v). The result is cached per `(machine_id, kind)` by the registry for
    /// the duration of the app session.
    fn is_available(&self, machine_id: &str) -> bool;

    /// The official install command, shown verbatim in the consent prompt.
    fn install_command(&self) -> &'static str;

    /// Spawn the agent and return a session handle. The session is fully
    /// initialized (capability negotiation, session/new, etc.) before this
    /// returns. Specific protocol-level work lives in concrete adapters.
    ///
    /// Async because the runtime may need to do network I/O during
    /// `initialize` / `session/new`; the return is a boxed future so
    /// the trait stays dyn-safe.
    fn start(
        &self,
        ctx: AgentContext,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn AgentSession>, AgentStartError>> + Send + '_>>;
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
