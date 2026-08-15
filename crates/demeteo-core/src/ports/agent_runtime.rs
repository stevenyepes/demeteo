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
use crate::domain::models::{Availability, EffortLevel, Platform, SessionInfo, WindowsAgentShell};
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
    /// The reasoning effort this turn asks for — already resolved through
    /// the precedence chain and already clamped to what the target agent
    /// supports. `None` means **inject nothing**: either the caller has no
    /// opinion, or the agent declares no effort levels at all (hermes).
    ///
    /// A peer of [`model`](Self::model), not a property of it. Each
    /// adapter's `ArgsBuilder` re-applies
    /// [`EffortLevel::clamp_for`](crate::domain::models::EffortLevel::clamp_for)
    /// against its own kind before emitting, so no caller needs per-agent
    /// knowledge and an unsupported level is unemittable.
    pub effort: Option<EffortLevel>,
    /// Optional step title, passed as `--title <value>` for CLI agents
    /// that support named sessions (opencode, hermes).
    pub title: Option<String>,
    /// The OS the agent process will actually run on, resolved through
    /// [`exec`](Self::exec) against [`machine_id`](Self::machine_id).
    ///
    /// Carried on the context rather than re-derived per adapter because
    /// `cfg!(windows)` is the wrong question in every one of them: the same
    /// desktop build spawns a local agent on Windows and a remote one on Linux
    /// within a single feature, so the answer belongs to the turn.
    ///
    /// `None` means the port could not name it, not "assume POSIX". An adapter
    /// that cannot proceed without knowing must emit nothing rather than guess
    /// — a POSIX-shaped flag sent to a Windows host is the failure this whole
    /// field exists to stop, and inventing one from a `None` reintroduces it
    /// silently.
    pub platform: Option<Platform>,
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
    /// Strip the machine-local personalization an agent would otherwise
    /// load — hooks, skills, extensions, themes, dynamic system-prompt
    /// sections — so the static prefix is byte-identical across worktrees
    /// and machines, which is what makes the provider prompt cache hit.
    ///
    /// A property of the *step*, not of the agent: `true` for every
    /// capability-scoped pipeline step, `false` for the interactive
    /// AgentTerminalDrawer and the model probe. An adapter whose CLI has no
    /// such switch ignores it. Never derive it from the agent kind — an
    /// adapter that grows those flags later is then silently left out of
    /// them, with nothing failing to say so.
    pub bare_mode: bool,
    /// Restrict which built-in tools are even *defined* for the session
    /// (claude-code: `--tools a,b`; `Some(vec![])` → `--tools ""`, no
    /// tools at all). Distinct from [`permissions`](Self::permissions),
    /// which *denies* tools but leaves their definitions in the model's
    /// context: an allowlist shrinks the prompt and removes the wasted
    /// turn where the model tries a tool that would only be denied. Use
    /// for single-purpose role turns (triage, finalize) whose entire job
    /// is to emit one structured answer. `None` = the agent's full set.
    /// Adapters without an equivalent flag ignore it.
    ///
    /// The names are **claude-code's** tool vocabulary (`Read`, `Grep`,
    /// `Glob`, …). An adapter whose CLI spells its tools differently must
    /// translate rather than forward: pi's are lowercase and it has no
    /// `glob`, so a verbatim `-t Read,Grep,Glob` selects nothing at all and
    /// the turn silently runs with no tools.
    pub tool_allowlist: Option<Vec<String>>,
    /// Hard ceiling on agentic turns for this session's process
    /// (claude-code: `--max-turns N`). Purely an anti-runaway guard:
    /// when tripped the CLI exits with an error result that still
    /// carries usage, and the step fails through the normal error path
    /// instead of burning tokens until the wall-clock watchdog fires.
    /// `None` = no cap. Adapters without an equivalent flag ignore it.
    pub max_turns: Option<u32>,
    /// Hard dollar ceiling on API spend for this session's process
    /// (claude-code: `--max-budget-usd N`). Like [`max_turns`](Self::max_turns)
    /// this is an anti-runaway guard, not a precise per-feature accountant:
    /// when tripped the CLI exits with an error result that still carries
    /// usage, and the step fails through the normal error path. The
    /// orchestrator derives the value from the resolved per-run budget
    /// (`Feature::max_budget_usd` → `ProjectSettings::default_max_budget_usd`
    /// → engine default), scaled per role. `None` = no cap. Adapters without
    /// an equivalent flag ignore it.
    pub max_budget_usd: Option<f64>,
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
/// HOME / USER / LOGNAME are **machine-aware** and resolved against
/// `machine_id`. The Tauri GUI's HOME and USER (`/home/<developer>`
/// and `<gui-user>` on the laptop) are meaningless to an agent
/// running on a remote box over SSH; opencode and claude-code both
/// read their config out of `$HOME`, and a wrong value causes the
/// agent to exit with code 1 and no useful diagnostic. Worse, a
/// split identity (`HOME=<remote>` but `USER=<gui>`) confuses some
/// provider auth flows. Both are asked of the execution port for
/// every machine, local included — `resolve_home` (the GUI process's
/// own home locally; on the SSH adapter, the value cached by its
/// first `printf %s "$HOME"` probe) and `resolve_user` (locally the
/// GUI's user; remotely the `Machine.username` the SSH channel
/// authenticates as). Being *resolved* rather than inherited is what
/// exempts HOME from the platform rule below — its value is asked of
/// the target machine, so it is already whatever that machine calls a
/// home, `USERPROFILE` included.
///
/// Which variables are inherited from the desktop at all is a decision,
/// not a list, and [`crate::domain::agent_env::inherited_agent_env`]
/// holds it along with the reasoning.
///
/// `ANTHROPIC_API_KEY` / `ANTHROPIC_AUTH_TOKEN` are intentionally **not**
/// injected here. Because we no longer pass `--bare` (which would set
/// `CLAUDE_CODE_SIMPLE=1` and disable keychain/OAuth reads), Claude Code
/// resolves and refreshes its own credential natively from the keychain
/// (macOS) or `~/.claude/.credentials.json` (all OSes). Demeteo handles
/// no Anthropic credentials at all. A user who exports
/// `ANTHROPIC_API_KEY=...` in their shell is still inherited and honored.
pub async fn agent_base_env(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
) -> HashMap<String, String> {
    let platform = resolve_agent_platform(exec, machine_id).await;
    let mut env: HashMap<String, String> = crate::domain::agent_env::inherited_agent_env(
        Platform::from_target_os(std::env::consts::OS),
        platform,
        |key| std::env::var(key).ok(),
    )
    .into_iter()
    .collect();
    let (home, user) = resolve_agent_identity(exec, machine_id).await;
    if !home.is_empty() {
        env.insert("HOME".to_string(), home);
    }
    if let Some(u) = user {
        env.insert("USER".to_string(), u.clone());
        env.insert("LOGNAME".to_string(), u);
    } else {
        // The port could not name the user (an unreachable remote, or a
        // GUI process launched without one). Forward whatever the parent
        // has rather than nothing: a POSIX agent with no `$USER` is worse
        // off than one carrying the desktop's.
        for key in ["USER", "LOGNAME"] {
            if let Ok(val) = std::env::var(key) {
                env.insert(key.to_string(), val);
            }
        }
    }
    env
}

/// Resolve the (HOME, USER) identity to forward to an agent process
/// spawned against `machine_id`, always through the execution port.
/// The local adapter reads the GUI process's own identity — and on
/// Windows that is `USERPROFILE` / `HOMEDRIVE`+`HOMEPATH` and
/// `USERNAME`, none of which a direct `std::env::var("HOME")` would
/// find; the SSH adapter returns its `home_cache` (probed via
/// `printf %s "$HOME"` over the channel) and the `Machine.username`
/// the channel authenticates as.
///
/// A remote machine that fails to resolve degrades to the *local*
/// home and no USER override: wrong, but the agent at least has some
/// `$HOME` rather than exiting 1 on a missing `~`. A local machine
/// that fails to resolve yields an empty home, which `agent_base_env`
/// drops rather than forwarding as `HOME=`.
pub async fn resolve_agent_identity(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
) -> (String, Option<String>) {
    let is_local = machine_id.is_empty() || machine_id == "local";
    let home = match exec.resolve_home(machine_id).await {
        Ok(h) if !h.is_empty() => h,
        _ if is_local => String::new(),
        _ => exec.resolve_home("local").await.unwrap_or_default(),
    };
    let user = exec.resolve_user(machine_id).await.ok();
    (home, user)
}

/// Resolve the platform to record on [`AgentContext::platform`] for a
/// session spawned against `machine_id`, always through the execution port.
///
/// Degrades to `None` on the same terms [`resolve_agent_identity`] drops the
/// USER: an unreachable machine, or a transport that declines to answer, must
/// not stop a turn that may not need the answer at all. It differs in having no
/// local fallback — the *machine's* OS is the whole question, so borrowing the
/// desktop's would answer a different one.
pub async fn resolve_agent_platform(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
) -> Option<Platform> {
    exec.resolve_platform(machine_id).await.ok()
}

/// Resolve just the HOME directory to forward as `$HOME` to an
/// agent process spawned against `machine_id`. Thin wrapper over
/// [`resolve_agent_identity`] for callers that don't need the
/// matching USER (most legacy call sites, plus the SSH adapter's
/// defense-in-depth override).
pub async fn resolve_agent_home(
    exec: &dyn crate::ports::execution::ExecutionPort,
    machine_id: &str,
) -> String {
    resolve_agent_identity(exec, machine_id).await.0
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

/// Turn the stdout of a runtime's model-listing command into model
/// identifiers, in the form they will be handed back to `--model`.
pub type ModelListParser = fn(output: &str) -> Vec<String>;

/// How to enumerate one runtime's selectable models.
///
/// Per-runtime rather than a shared `<binary> models` convention because
/// getting it wrong is not a no-op: a CLI with no such subcommand parses
/// `models` as a **prompt**, spends a turn answering it, and hands prose to
/// the parser. Two agents happen to share the subcommand; that is a
/// coincidence, not a contract.
#[derive(Debug, Clone, Copy)]
pub struct ModelListing {
    /// Appended to the runtime's binary, verbatim, as the shell command.
    pub args: &'static str,
    pub parse: ModelListParser,
}

impl ModelListing {
    /// `<binary> models`, one identifier per line — opencode and hermes.
    pub const MODELS_SUBCOMMAND: Self = Self {
        args: "models",
        parse: models_one_per_line,
    };
}

/// Every non-blank line is one model identifier.
pub fn models_one_per_line(output: &str) -> Vec<String> {
    output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

/// What Demeteo's own spawn flags do to the machine-local personalization a
/// harness would otherwise load — the user's skills, commands, prompt
/// templates, themes and settings files.
///
/// The subject is Demeteo's argv, never the harness's feature set. Which
/// review capability a given harness ships, and what it is called there, is
/// another product's vocabulary: it changes on their release schedule, nothing
/// here fails when it does, and the stale claim reads as authoritative
/// forever. What [`AgentContext::bare_mode`] strips is ours to know, and each
/// adapter's `build_args` is the whole evidence for its value.
///
/// Declared beside [`AgentCapabilities::effort_levels`] for that field's own
/// reason: the frontend states the consequence to the user without keeping a
/// per-agent list of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PersonalizationSupport {
    /// Under `bare_mode` the user's own setup is still loaded.
    Loaded,
    /// Under `bare_mode` the harness is told to switch it off, so a
    /// capability-scoped step runs without it. The one value that costs the
    /// user something they would otherwise have had.
    Suppressed,
    /// The adapter reads no `bare_mode` at all: whatever the harness loads
    /// unprompted on that machine is what the step gets. Distinct from
    /// [`Loaded`](Self::Loaded), which is a switch Demeteo holds and chose not
    /// to throw.
    Native,
}

/// The capabilities Demeteo asks of a coding agent, declared once per runtime
/// instead of being inferred from `match kind { ... }` string lists scattered
/// across the executor, the model probe, and the UI.
///
/// Adding a new agent means filling this in (plus `parse_event` / `build_args`
/// / `perm_env`) — no downstream site special-cases the kind. See
/// `docs/adapters/CONTRIBUTING-AN-AGENT.md`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCapabilities {
    /// Human-facing name for pickers and settings (e.g. "Claude Code").
    /// Replaces the ad-hoc `kind.replace('-', ' ')` / display-name maps in
    /// the frontend.
    pub display_label: &'static str,
    /// Whether the CLI can enumerate its models at all. Mirrors
    /// [`model_listing`](Self::model_listing)`.is_some()` as a plain bool
    /// for the UI, which has no use for the command itself.
    pub lists_models: bool,
    /// The command and parser `application::agent_probe` uses for dynamic
    /// model discovery. `None` → the agent falls back to a static list.
    ///
    /// Skipped by serde in both directions: a fn pointer has no wire form,
    /// and this is a backend contract the UI never reads.
    #[serde(skip)]
    pub model_listing: Option<ModelListing>,
    /// The model this runtime uses when no explicit override is configured,
    /// used to seed `UsageAccumulator` for pricing-table cost fallback.
    /// `None` when the default isn't statically knowable.
    pub default_model: Option<&'static str>,
    /// The effort levels this agent actually accepts per invocation, in
    /// ladder order. Drives the UI picker, so a harness with no
    /// per-invocation effort control (hermes) declares `&[]` and the
    /// control greys out instead of the frontend hardcoding a per-agent
    /// list. Mirrors
    /// [`EffortLevel::supported_for`](crate::domain::models::EffortLevel::supported_for).
    ///
    /// Serializes out to the UI; `skip_deserializing` because serde has no
    /// `Deserialize` for a borrowed slice of non-`u8` — the value is always
    /// declared in Rust, never read back off the wire.
    #[serde(skip_deserializing)]
    pub effort_levels: &'static [EffortLevel],
    /// What this harness's personalization does under
    /// [`AgentContext::bare_mode`], which every capability-scoped step sets.
    /// Read the adapter's `build_args` before changing a value here; the type's
    /// own docs carry why it is a claim about Demeteo and not about the harness.
    pub personalization: PersonalizationSupport,
    /// The interpreter this harness runs agent-authored commands under on
    /// Windows, which decides what the platform block may promise about
    /// command syntax. Read through
    /// [`AgentRegistry::windows_agent_shell_for`](crate::adapters::agent::registry::AgentRegistry::windows_agent_shell_for);
    /// the type's own docs carry why it is declared here and not probed.
    pub windows_agent_shell: WindowsAgentShell,
}

/// Transport-neutral runtime for a single agent. The runtime takes a binary
/// and a config and owns the lifecycle of one agent session: spawning,
/// initialization, prompt streaming, cancel, and clean teardown.
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    /// Stable identifier; matches `AgentConfig.kind` and
    /// [`AgentKind::as_str`](crate::domain::models::AgentKind::as_str).
    fn kind(&self) -> &'static str;

    /// The capabilities Demeteo asks of this agent. Every behavior decision
    /// that used to `match` on the kind string reads a field here instead.
    fn capabilities(&self) -> AgentCapabilities;

    /// The actual executable name on disk. Defaults to [`Self::kind`], which
    /// is correct when the kind matches the binary (opencode, hermes, …).
    /// Runtimes whose kind is a hyphenated label that doesn't exist as a
    /// binary on `$PATH` (e.g. `claude-code` kind → `claude` binary) must
    /// override this so the executor spawns the right process.
    fn binary(&self) -> &'static str {
        self.kind()
    }

    /// Check whether the binary is reachable on the target host (which /
    /// command -v). A conclusive result is cached per `(machine_id, kind)` by
    /// the registry for the duration of the app session.
    ///
    /// Report [`Availability::Unknown`] when the probe itself could not be
    /// run — the host was unreachable, the transport errored. Answering
    /// `Missing` there is what turns a dropped SSH connection into a stored
    /// "user disabled this agent"; see the type's own docs.
    async fn availability(
        &self,
        exec: &dyn crate::ports::execution::ExecutionPort,
        machine_id: &str,
    ) -> Availability;

    /// The official install command, shown verbatim in the consent prompt.
    fn install_command(&self) -> &'static str;

    /// The model this runtime selects when no explicit override is configured.
    /// Used to seed `UsageAccumulator` so the pricing-table fallback can
    /// compute `cost_usd` even when the agent's wire format omits it.
    ///
    /// Delegates to [`AgentCapabilities::default_model`]; runtimes declare the
    /// value there rather than overriding this.
    fn default_model(&self) -> Option<String> {
        self.capabilities().default_model.map(str::to_string)
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

    /// The working directory this session is bound to (the `--dir` /
    /// cwd it was spawned with). CLI agents resume against the
    /// directory the session was *created* in, not the `--dir` passed
    /// on a later turn — so a registry-cached session whose `cwd()`
    /// differs from the current step's worktree would write the step's
    /// deliverable into the wrong (earlier, now-ephemeral) worktree.
    /// `spawn_agent_session` compares this against the step's worktree
    /// and respawns fresh on a mismatch. Default `""` = "not tracked",
    /// which suppresses the guard for runtimes without a bound cwd
    /// (NoopRuntime, stubs).
    fn cwd(&self) -> &str {
        ""
    }
}

/// Cheaply-cloneable handle that tracks how recently the agent's stderr
/// produced output. The stderr drain thread calls [`beat`](StderrHeartbeat::beat) on every
/// line; the step executor polls [`last_activity_ago_ms`](StderrHeartbeat::last_activity_ago_ms) to decide
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

    /// Milliseconds since the last call to [`beat`](StderrHeartbeat::beat) (or since construction
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

#[cfg(test)]
#[path = "../../tests/unit/agent_base_env.rs"]
mod tests;
