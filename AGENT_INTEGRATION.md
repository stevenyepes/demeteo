# Agent Integration Spec (v1, post-pivot)

This document is the **source of truth for how Demeteo integrates coding
agents** in the multi-agent orchestrator. It captures the runtime trait,
the `AcpRuntime` implementation, and the *narrowed* surface that flows
from the pivot: the `AcpRuntime` is no longer called by a per-thread UI
stream, but by the `StepExecutor` (one agent session per step execution).

The pivot's locked decisions are in [`docs/REDESIGN_DECISIONS.md`](docs/REDESIGN_DECISIONS.md).
The full architecture is in [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md).
The master plan is in [`REDESIGN_PLAN.md`](REDESIGN_PLAN.md).

For the surrounding architecture (hexagonal layout, plugin host, port
trait catalogue), see [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md).

---

## 1. Scope and Non-Goals (post-pivot)

### v1 ships

- A pluggable `AgentRuntime` trait with one concrete implementation: `AcpRuntime` (JSON-RPC over stdio, per the [Agent Client Protocol](https://agentclientprotocol.com/) v1).
- Two agent configurations out of the box: **`hermes`** and **`opencode`** (the `anomalyco/opencode` project, the open-source coding agent — *not* the archived `opencode-ai/opencode`).
- The runtime serves **both** the planner (an agent session that decomposes the feature into a step DAG) and the subtask agents (sessions that execute a single `agent` or `parallel` step's work). Same trait, same plumbing, different prompts.
- Lazy agent session lifecycle scoped to step executions (a session is created on first prompt for a step, torn down on step completion).
- A scope fence in the policy engine that auto-rejects any file path resolving outside the worktree. (The worktree is now per-subtask, branched off the feature branch.)
- A typed three-layer error model (per-action / per-step / per-feature).
- Per-step checkpoint persistence (DB-backed, populated on every state transition).
- The `AgentEvent` vocabulary is **internal** — consumed by the `StepExecutor`, not by the UI. The UI sees step transitions, not agent transcripts.

### v1 explicitly does NOT include

- **Secret management for the brain's API keys.** The user pre-configures their agent (provider, API key, model) on the host where the agent runs. Demeteo does not read, store, or inject model API keys. **The planner is just another agent session in this respect.** Phase 8+ candidate.
- **A demeteo-side LLM.** The "brain" is a coding agent (opencode or hermes) invoked by the `StepExecutor`. Demeteo never calls a model provider directly.
- **Resume / context replay across restarts.** A Feature Run is a C-strict opaque cursor; the agent session id is internal. The orchestrator *does* re-enter a feature at the last completed step on launch (synthetic gate on mid-step interrupt), but it does not replay prior agent transcripts to the new session. The step's *artifact* is the cross-restart state.
- **Non-ACP transports.** The `AcpRuntime` is the only runtime in v1. Adding HTTP/Server / stdio JSON-lines / MCP-as-agent transports is v1.1 and explicitly chosen to be a *different* protocol so the abstraction gets a real test.
- **WASM policy plugins.** Designed in `docs/LEGACY_ARCHITECTURE.md`; deferred. The policy decorator + scope fence + per-project conflict policy cover v1 approval needs.
- **Per-agent settings UI (model picker, working dir override, etc.).** The user configures their agent on the host. Demeteo doesn't expose a per-agent settings surface in v1. v1.1 candidate.
- **Auto-restart on transient errors.** Single restart on user request only.
- **Token/cost usage dashboard.** The `Usage` event is wired into the protocol stack but the UI surfaces per-step cost from the `PricingTable`, not a token counter. A v1.x polish item.
- **A chat-style supervisor UI.** The chat UX is gone (per the pivot). The UI is a fleet-control surface; the agent's own chat is not demeteo's concern.
- **Working memory.** No chat, no working memory sidecar. The per-step artifact is the durable record.

### Why "ACP only" is the right bet for v1

In mid-2026 the two leading open-source coding agents (Hermes and anomalyco/opencode) both ship **ACP as a first-class, documented surface** in their main navigation. ACP is JSON-RPC 2.0 over stdio (local) or Streamable HTTP / WebSocket (remote), with stable v1 wire format, official SDKs for Rust/Python/TS/Java/Kotlin, and an active RFD process. The bet is not "will ACP be adopted" — it's "every serious agent in this category speaks ACP or will have to."

We design the runtime trait to be transport-neutral so we can add non-ACP runtimes later, but the v1 surface area is intentionally narrow: one trait, one implementation, two configs. This keeps the `StepExecutor`, the policy decorator, the channels, and the orchestrator ignorant of which agent is in use. The "second adapter must be non-ACP" rule from the legacy design interview is a v1.1 commitment to validate the abstraction, not a v1 requirement.

---

## 2. Locked Decisions (the runtime-relevant ones)

Cross-reference: full table in [`docs/REDESIGN_DECISIONS.md`](docs/REDESIGN_DECISIONS.md).

| #  | Decision                           | Section here |
|----|------------------------------------|--------------|
| 1  | Top-level entity shape             | §3.1         |
| 2  | Demeteo's role                     | §0 (preamble)|
| 3  | Brain role                         | §1 (scope)   |
| 4  | LLM provider scope                 | §1 (scope)   |
| 5  | Planner selection                  | §3.2         |
| 6  | Project structure                  | §3.3         |
| 8  | Step execution model               | §3.4, §4     |
| 13 | `parallel` failure semantics       | §4.3         |
| 14 | Workflow re-entry / resume         | §3.5         |
| 16 | Repo merge model                   | §3.6         |
| 17 | PAT scope                          | §3.3         |
| 20 | Conflict resolution UX             | §4.4         |

---

## 3. Domain Model (post-pivot)

### 3.1 The agent session is scoped to a step execution

`StepExecution` (`src-tauri/src/domain/feature.rs`) is the new top-level agent-session owner:

```rust
pub struct StepExecution {
    pub id: String,
    pub feature_run_id: String,
    pub step_index: u32,
    pub step_type: String,         // "agent" | "parallel" | "gate"
    pub status: String,            // see §3.5
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub wall_clock_seconds: Option<u64>,
    pub cost_usd: Option<f64>,
    pub agent_kind: Option<String>,    // "opencode" | "hermes" | None (for gates)
    pub agent_session_id: Option<String>,  // internal; never crosses the IPC boundary
    pub artifact_paths: Vec<String>,
    pub gate_decision_id: Option<String>,  // Some(...) iff this is a gate step
}
```

The agent *session* identifier (ACP's `sessionId`) is owned by the `AgentRegistry` and never crosses the Rust↔TypeScript boundary. From the orchestrator's perspective, a step execution *is* the session. The ACP `sessionId` is allowed to change (auto-compact, reconnect) and if the UI held it, we'd have a synchronization nightmare.

**No resume across restarts at the session level.** A restarted Demeteo finds the `step_executions` row in SQLite; if it was `running`, the orchestrator marks it `interrupted` and surfaces a synthetic gate (see §3.5). The next directive (i.e., the user clicking "Resume" or the orchestrator continuing) creates a fresh agent session for the step. The step's *artifact* is the cross-restart state.

### 3.2 The planner is just an agent session

There's no special "planner port" or "planner runtime." The planner is a coding agent session (opencode or hermes) invoked with a *planning prompt* — the same `AcpRuntime`, the same transport, the same tool bridge. The only special thing is the prompt template, which lives in the workflow step's config (the first `agent` step in the starter pack's Research → Spec → Plan → Tasks → Implement → Validate workflow, for example).

The planner's selection is per-project (`Project.planner: { machine_id, agent_kind }` — see `ProjectRepository` in [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md) §2). The user picks the planner when creating the project. The orchestrator resolves it at feature-start time.

### 3.3 Project host + provider instance

Each project has exactly one host (`Project.host: { type: "local" | "remote", ... }`). A project is bound to a provider instance at creation; the instance's PAT is used for both `git clone` (via `SshRepositoryCloner` / `LocalFsRepositoryCloner`) and `mr_publish` (via `MrPublisher`). The provider instance is keyed by `(kind, host)` to support multiple GitLab instances and GitHub Enterprise Server.

The agent runs on the **same host as the worktree**:
- `auth_type == "local"` → `tokio::process::Command` with the user's shell env inherited.
- `auth_type in {"key", "password", "agent"}` → SSH channel via `ExecutionPort::spawn_interactive`. Demeteo connects over the existing authenticated SSH session, runs the agent binary over a long-lived `ssh2::Channel`, and owns both ends of the stdio.

**No per-machine override.** The location is implied by the project's host. One less way to misconfigure.

### 3.4 The `AgentEvent` vocabulary is internal

`src-tauri/src/domain/agent_event.rs`:

```rust
use serde::{Deserialize, Serialize};
use crate::domain::policy::ActionKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streamed assistant text delta. The StepExecutor appends to the current
    /// artifact buffer; not surfaced to the UI.
    Text { delta: String },

    /// Agent wants to do something. The `tool_call_id` is the agent's id; the
    /// `intercept_id` is Demeteo's internal handle.
    ToolCall {
        tool_call_id: String,
        intercept_id: String,
        action: ActionKind,
        target: String,
        preview: Option<String>,
    },

    /// In-flight tool call update (status change, refreshed diff, etc.)
    ToolCallUpdate {
        tool_call_id: String,
        status: ToolCallStatus,
        preview: Option<String>,
    },

    /// Token / cost telemetry
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
    },

    /// Soft error from the agent
    Error {
        code: String,
        message: String,
        recoverable: bool,
    },

    /// Agent finished the turn. The channel closes after this.
    TurnComplete { stop_reason: StopReason },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    InProgress { message: Option<String> },
    Completed,
    Failed { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndOfTurn,
    Cancelled,
    MaxTokens,
    Error,
}
```

The `Text` and `Plan` events are consumed by the `StepExecutor` to build the step's artifact (per the type-driven defaults in `REDESIGN_PLAN.md` §1 decision 28). The `Usage` event feeds the per-step `cost_usd` and the per-feature telemetry (per the locked decision 15). The `TurnComplete` event drives the step's state transition.

The UI does **not** consume the agent event stream. It consumes `feature_status_changed` / `step_progress` / `gate_required` / `conflict_detected` events from the `NotificationPort`.

### 3.5 Step status state machine

`StepExecution.status` values:

| Value             | Meaning                                              | Set by                                |
|-------------------|------------------------------------------------------|---------------------------------------|
| `pending`         | Step is next up, not yet started                     | `StepExecutor` on run start           |
| `running`         | Agent session is active                              | `StepExecutor` on first AgentEvent    |
| `awaiting_gate`   | A gate is awaiting user decision                     | `StepExecutor` on `gate_required`     |
| `completed`       | Step finished; artifact written                      | `StepExecutor` on `TurnComplete`      |
| `failed`          | Step failed; user action required                    | `StepExecutor` on terminal Error      |
| `skipped`         | Step was skipped (e.g., conflict resolution skip)    | `StepExecutor` on user skip           |
| `interrupted`     | App was killed mid-step; synthetic gate on re-entry  | `StepExecutor` on shutdown watchdog   |

Per-step checkpoints (decision 14) are atomic: a step transitions to `completed` only when its artifact is written and (if it's a gate) its `gate_decision` is recorded. Mid-step crashes surface as `interrupted`, and the next launch offers a synthetic gate with "Resume" (re-run the step) or "Skip" options.

### 3.6 The worktree-of-record is `feature/<slug>`

Subtask worktrees branch off `feature/<slug>` (decision 16). The orchestrator creates `feature/<slug>` off the project's canonical branch at feature start. Each subtask's worktree is branched off the *latest* `feature/<slug>` (i.e., after any prior subtask merges). Subtask branches merge into `feature/<slug>` in topological DAG order via the `MergeExecutor`.

The scope fence (§4.2) is evaluated per-subtask against the subtask's worktree root. The user's `feature/<slug>` branch is touched only at merge time, never by an agent directly.

---

## 4. Runtime Trait and Lifecycle

### 4.1 The trait

`src-tauri/src/ports/agent_runtime.rs`:

```rust
use std::pin::Pin;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::Stream;
use serde::{Deserialize, Serialize};

use crate::domain::agent_event::AgentEvent;

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub step_execution_id: String,  // NEW: scoped to a step, not a thread
    pub feature_run_id: String,
    pub machine_id: String,
    pub binary: String,        // resolved absolute path
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub cwd: String,
}

#[derive(Debug, thiserror::Error)]
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

pub trait AgentRuntime: Send + Sync {
    fn kind(&self) -> &'static str;

    fn is_available(&self, machine_id: &str) -> bool;

    fn install_command(&self) -> &'static str;

    fn start(&self, ctx: AgentContext) -> Result<Arc<dyn AgentSession>, AgentStartError>;
}

pub trait AgentSession: Send + Sync {
    fn session_id(&self) -> &str;

    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

    fn cancel(&self) -> Result<(), String>;
}
```

The `step_execution_id` field on `AgentContext` is the only material change from the legacy spec — it scopes the session to a step, not a thread. The `AgentRegistry` (in `adapters/agent/registry.rs`) holds the `HashMap<StepExecutionId, Arc<AgentSession>>` and is the only thing that knows which step executions have live agents.

### 4.2 Lifecycle: lazy on first prompt, scoped to step execution

A `StepExecution` row exists in SQLite before any agent does. The agent session is created at the moment the executor submits the step's first prompt, and torn down on:

- **Step completion** (terminal `TurnComplete` or terminal `Error`): clean teardown, session removed from registry.
- **Step failure** (terminal `Error`): clean teardown, working state preserved.
- **Step retry** (per Q14 retry policy): the previous session is killed; a new session is spawned.
- **Feature pause / cancel / re-run**: all live sessions for the feature are killed.
- **App shutdown**: all live sessions are killed; mid-step rows transition to `interrupted`.

The `AgentRegistry` is the only thing that knows which step executions have live agents. When a feature is paused or the app shuts down, the registry iterates and calls `kill()` on each session.

### 4.3 Where the process lives

The agent process runs on the **same host as the worktree** (the project's host, per §3.3):

- `auth_type == "local"` → `tokio::process::Command` with the user's shell env inherited.
- `auth_type in {"key", "password", "agent"}` → SSH channel via the existing `ExecutionPort::spawn_interactive`. Demeteo connects over the existing authenticated SSH session, runs the agent binary over a long-lived `ssh2::Channel`, and owns both ends of the stdio.

**No per-machine override.** The location is implied by the project's host. One less way to misconfigure.

### 4.4 The transport layer

The `AgentRuntime` operates on `bytes-in, bytes-out`. The transport layer is a separate concern, factored out as `AgentTransport` so the runtime is transport-blind:

```rust
pub trait AgentTransport: Send {
    fn stdin(&mut self) -> &mut dyn std::io::Write;
    fn stdout(&mut self) -> &mut dyn std::io::Read;
    fn kill(&mut self) -> Result<(), String>;
    fn try_wait(&mut self) -> Result<Option<i32>, String>;
}
```

Two implementations:
- `LocalSubprocessTransport` — wraps `tokio::process::Child`.
- `RemoteSshTransport` — wraps an `ssh2::Channel` held open by `SshClientAdapter::spawn_interactive`.

`AcpRuntime` takes an `AgentTransport` and produces an `AgentEvent` stream by parsing the transport's stdout as JSON-RPC and mapping ACP methods onto `AgentEvent` variants.

### 4.5 Conflict resolution cascade (decision 20)

When a `parallel` step's subtask merge conflicts with the `feature/<slug>` branch, the `MergeExecutor` produces a `ConflictReport` and the `ConflictResolver` cascade kicks in:

1. **Auto-agent** (default `conflict_policy: "auto_agent"`): spawn a conflict-resolution subtask — a fresh agent session with a constrained prompt ("resolve the conflicts in these N files; do not modify unrelated code; produce a resolution commit"). Cost-capped (default 2 attempts, $0.50).
2. **Manual** (on auto-agent failure or `conflict_policy: "auto_human"`): open the `ConflictResolver` UI (Monaco 3-way merge).
3. **Skip / Abort** (always available): mark the subtask `skipped` or the feature `aborted`.

The cascade is enforced by the `StepExecutor` and `ConflictPolicy` (per-project setting). See [`docs/REDESIGN_OPEN_QUESTIONS.md`](docs/REDESIGN_OPEN_QUESTIONS.md) §1 for the deferred per-step retry policy.

---

## 5. The AcpRuntime

### 5.1 Why ACP

[Agent Client Protocol](https://agentclientprotocol.com/) v1 is the standard protocol for agent↔client communication. It's JSON-RPC 2.0 over stdio (local) or Streamable HTTP / WebSocket (remote). v1 wire format is stable. Official SDKs exist for Rust (`agent-client-protocol` crate), Python, TypeScript, Kotlin, Java.

Both target agents in v1 speak ACP as a first-class feature:
- **Hermes** has a dedicated `acp_adapter/` module and `acp_registry/` in its repo.
- **anomalyco/opencode** lists **ACP Support** in the main navigation of its docs site (`opencode.ai/docs/acp/`).

### 5.2 Method surface (v1 subset)

We implement the *client* side (Demeteo is the ACP client, the agent is the ACP server). v1 needs:

| Method                     | Direction         | Maps to                                            |
|----------------------------|-------------------|----------------------------------------------------|
| `initialize`               | client → agent    | one-time capability negotiation                    |
| `authenticate`             | client → agent    | only if agent advertises `authMethods`             |
| `session/new`              | client → agent    | creates the per-step session; returns `sessionId`  |
| `session/prompt`           | client → agent    | sends user directive; response is a stream         |
| `session/cancel`           | client → agent    | stops in-flight turn                                |
| `session/set_mode`         | client → agent    | optional: switch build/plan modes                  |
| `fs/read_text_file`        | agent → client    | delegated to `PolicyEnforcedExecutionPort::read_file` |
| `fs/write_text_file`       | agent → client    | delegated to `PolicyEnforcedExecutionPort::write_file` |
| `terminal/create`          | agent → client    | delegated to `ExecutionPort::run_command`           |
| `terminal/output`          | agent → client    | streamed                                            |
| `terminal/wait_for_exit`   | agent → client    | blocks                                              |
| `terminal/release`         | agent → client    | cleanup                                             |

Agent-initiated notifications we listen for:

| Notification              | Maps to                          |
|---------------------------|----------------------------------|
| `session/update` (text)   | `AgentEvent::Text`               |
| `session/update` (tool_call) | `AgentEvent::ToolCall`        |
| `tool_call/update`        | `AgentEvent::ToolCallUpdate`     |
| `session/usage_update`    | `AgentEvent::Usage`              |
| end of stream             | `AgentEvent::TurnComplete`       |

### 5.3 Install flow

The project's `EnvModal` (or its successor `ProviderSettings`) lets the user toggle `enabled` for each `AgentConfig` on the project's host. When the `StepExecutor` needs to spawn a step on a machine, it checks availability:

```
step_executor.spawn_step(step_execution_id, agent_kind)
  → registry.spawn(kind, ctx)
  → runtime.is_available(machine_id) ?
       yes → spawn
       no  → return AgentStartError::NotFound(binary_name) + install_command
```

On `NotFound`, the UI shows a consent modal with the install command shown verbatim:

> **Install opencode on `spectacular`?**
> The following official script will be run via SSH:
> ```
> curl -fsSL https://opencode.ai/install | bash
> ```
> [Cancel] [Install and continue]

On consent, the frontend invokes `agent_install_and_start(step_execution_id, agent_kind)` which:
1. Runs the install command over the appropriate transport (local shell or SSH).
2. Re-checks availability.
3. If available, spawns the agent and returns the session handle.
4. If still not found after install, returns an error and the step is left in `error` state.

**Eager on first step per machine, lazy after.** The result of the availability check is cached per `(machine_id, agent_kind)` for the duration of the app session. If the user later uninstalls the agent, the lazy fallback (the spawn itself fails with ENOENT) re-triggers the install flow.

**No user-editable install commands.** The command is static per adapter, baked into the source. The user can only consent or cancel.

### 5.4 The tool bridge

When the agent sends `fs/read_text_file { path }`, the runtime doesn't have direct filesystem access. It calls into the `tool_bridge.rs` module, which:

1. Resolves `path` against the step's worktree (absolute path, no `..`, symlinks resolved via the existing `SftpEntry` metadata).
2. Calls `PolicyEnforcedExecutionPort::submit` with `AgentAction::Read { path: resolved }`.
3. The policy engine runs the existing rules **plus the scope fence pre-rule** (§6).
4. If approved (or auto-approved by rule), the file is read via the underlying `ExecutionPort` and the result is returned to the agent.
5. If rejected (by rule, by scope fence, or by user), a `tool_call/update { status: Failed, content: [{type: "text", text: reason}] }` is sent to the agent.

The scope fence (§6) is the path-only pre-rule that runs *before* any user rule. The worktree of record is the *subtask's* worktree, branched off `feature/<slug>` (per §3.6). Every file operation the agent attempts is subject to the same policy and scope checks.

### 5.5 Adapters (v1)

Two agents, both via `AcpRuntime`:

- `adapters/agent/opencode/mod.rs` — wraps `AcpRuntime` with config `{ binary: "opencode", args: ["acp"], env: {}, install_command: "curl -fsSL https://opencode.ai/install | bash" }`.
- `adapters/agent/hermes/mod.rs` — wraps `AcpRuntime` with config `{ binary: "hermes", args: ["acp"], env: {}, install_command: "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash" }`.

The `AcpRuntime` itself is generic over the binary — the agent-specific logic is just the `AgentConfig` and the availability check (which binary name to look up).

### 5.6 Disclaimer

Both the README and the `ProviderSettings` UI strings should clarify that Demeteo's `opencode` integration targets `anomalyco/opencode` (the open-source coding agent project) and that Demeteo is **not affiliated with the opencode project**. The `anomalyco/opencode` README explicitly asks projects using "opencode" in their name to make this clear.

---

## 6. The Scope Fence

### 6.1 The problem

Without a fence, an agent running with `cwd = <worktree>` can `cd ..` out of the worktree, read `/etc/passwd`, write to `~/.ssh/authorized_keys`, etc. The policy decorator catches writes and bash, but the bash-prefix match isn't a *path* check — `cat /etc/passwd` slips through unless the user has a `Reject` rule for `cat /etc/*`.

The scope fence is a *path-only* pre-rule that runs before any user rule.

### 6.2 The implementation

Extend `src-tauri/src/domain/policy_engine.rs` with a pre-rule check. The pre-rule is constructed at policy-compile time from the step's worktree and is evaluated *first* on every `AgentAction`:

```rust
pub struct ScopeFence {
    pub worktree_path: PathBuf,   // the subtask's worktree root
}

impl ScopeFence {
    pub fn check(&self, action: &AgentAction) -> Option<PolicyDecision> {
        let target_path = match action {
            AgentAction::Read { path }
            | AgentAction::Edit { path, .. }
            | AgentAction::Write { path, .. } => path,
            AgentAction::RunBash { .. } => return None, // bash handled by prefix policy
        };

        let resolved = match resolve_path(&self.worktree_path, target_path) {
            Ok(p) => p,
            Err(_) => return Some(PolicyDecision::Reject {
                reason: "path resolution failed".into(),
            }),
        };

        if !resolved.starts_with(&self.worktree_path) {
            return Some(PolicyDecision::Reject {
                reason: format!("path '{}' is outside step scope", target_path),
            });
        }

        None
    }
}
```

`PolicyEngine::evaluate` is updated to call `ScopeFence::check` first. A returned `Some(decision)` short-circuits the user rules. Bash actions return `None` from `check`, so the scope fence is *invisible* for bash and the existing prefix policy still applies.

### 6.3 Path resolution

`resolve_path(worktree, target)`:
- If `target` is absolute, canonicalize it (resolving symlinks via the existing SFTP `get_metadata`).
- If `target` is relative, join with `worktree`, then canonicalize.
- Return `Err` if the path doesn't exist *or* if canonicalization fails (we want to fail closed).

For `Edit` and `Write` actions, the target is the *destination*, not a path being read. The fence applies to the destination the same way.

### 6.4 Overriding the fence

The fence returns `Reject`, not `EscalateToUser`. So a user rule with `Approve` for a specific outside-scope path *won't* override it — the fence is evaluated first and short-circuits.

The escape hatch for power users: a new `source` on `PolicyRule` gains a new value: `"scope_override"`. The policy engine, when compiling rules, **skips the scope fence for any rule with `source = "scope_override"`**. The `PolicyEditor` UI gets a "Scope override" toggle on a rule row that's disabled by default.

### 6.5 What this does NOT solve

- **Bash command scope.** `RunBash { cmd: "cat /etc/passwd" }` is still subject to the bash-prefix policy, not the scope fence. The user writes rules like `Reject("cat /etc/*")` for that.
- **TOCTOU.** Symlink races between resolution and execution are best-effort, not bulletproof. The existing policy layer was never TOCTOU-tight and this doesn't change that.
- **Cross-worktree access in the feature branch.** The scope fence protects the subtask's worktree. A subtask that *should* see prior subtask merges gets them via the merge into `feature/<slug>` (not via a separate read path).

---

## 7. Error Model

### 7.1 Per-action errors (typed)

`ActionError` (carried from v1, unchanged in shape):

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionError {
    Network { message: String },
    Permission { message: String },
    NotFound { message: String },
    Policy { message: String },
    Internal { message: String },
}
```

The frontend maps each variant to a small set of recovery chips (per the legacy v1 spec). The Tauri commands that return this are: `project_*`, `workflow_*`, `feature_*`, `step_*`, `gate_*`, `merge_*`, `conflict_*`, `mr_*`, `sftp_*`, `test_ssh_connection`, etc. Existing free-form `Err` returns are migrated incrementally — the rule is "every new error path returns `ActionError`."

### 7.2 Per-step errors (`AgentEvent::Error`)

The agent emits structured errors. The `StepExecutor` consumes them and transitions the `StepExecution.status` to `failed` (terminal) or stays in `running` (recoverable). The UI renders the failed step in the `FeatureDetail` step timeline with the agent's error code + message, styled with the design-system ruby accent (`AGENTS.md` §2). If `recoverable: true`, the user has a "Retry step" affordance (per the opt-in retry policy). If `recoverable: false`, the user has "Skip" or "Abort feature."

### 7.3 Per-feature errors (watchdog)

The `AgentTransport::try_wait` is polled by a watchdog task per active step execution. When the underlying process exits (or the SSH channel closes), the watchdog:

1. Sets `StepExecution.status = "failed"` with a reason.
2. Drains any pending gate decisions for the step with `Resolution::Reject { feedback: "agent process exited" }`.
3. Emits `feature_status_changed { feature_id, status: "step_failed" }`.
4. Surfaces the failure to the user in `FeatureDetail` with a "Retry step / Skip / Abort feature" affordance.

---

## 8. Tauri Command Surface (post-pivot)

The full list of new commands is in [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md) §4. The runtime-relevant commands (the ones the `StepExecutor` and `FeatureOrchestrator` invoke on the agent runtime):

```rust
// Phase R5
#[tauri::command]
async fn agent_start(
    state: tauri::State<'_, AgentRegistryState>,
    step_execution_id: String,
    agent_kind: String,
) -> Result<AgentStartResult, String>;

#[tauri::command]
async fn agent_install_and_start(
    state: tauri::State<'_, AgentRegistryState>,
    step_execution_id: String,
    agent_kind: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_prompt(
    state: tauri::State<'_, AgentRegistryState>,
    step_execution_id: String,
    text: String,
) -> Result<(), String>;  // prompt is enqueued; events flow via NotificationPort

#[tauri::command]
async fn agent_cancel(
    state: tauri::State<'_, AgentRegistryState>,
    step_execution_id: String,
) -> Result<(), String>;
```

The legacy `agent_prompt` returned a `Channel<AgentEvent>` for per-turn UI streaming. In the post-pivot design, the prompt is enqueued on the `StepExecutor`'s internal channel; the UI never subscribes to per-turn streams. The `StepExecutor` consumes the `AgentEvent` stream and emits `step_progress` / `feature_status_changed` / `gate_required` events that the UI does subscribe to.

---

## 9. File Layout (post-pivot)

The files touched by the agent integration work, organized by phase. Existing files modified are marked with `(modified)`; new files are marked with `(new)`.

### Phase R1 — Port + domain skeleton

```
src-tauri/src/domain/
  agent_event.rs                  (modified) AgentEvent vocabulary (Text/ToolCall/Usage/TurnComplete; Plan dropped)
  models.rs                       (modified) add StepExecution, GateDecision, ConflictReport

src-tauri/src/ports/
  agent_runtime.rs                (modified) AgentContext adds step_execution_id
  db.rs                           (modified) extend DatabasePort with new tables

src-tauri/src/adapters/database/
  sqlite.rs                       (modified) implement new tables; legacy thread_sessions preserved
```

### Phase R4 — Step executor + AcpRuntime

```
src-tauri/src/adapters/agent/
  mod.rs                          (modified) declare registry, acp, opencode, hermes submodules
  registry.rs                     (modified) HashMap<StepExecutionId, Arc<AgentSession>>
  acp/
    mod.rs                        (new)
    runtime.rs                    (new) AcpRuntime: spawns agent, owns transport, drives ACP session
    event_mapper.rs               (new) ACP notifications -> AgentEvent
    tool_bridge.rs                (new) ACP fs/* + terminal/* -> PolicyEnforcedExecutionPort
    install.rs                    (new) run_official_install() over local or SSH transport
  opencode/
    mod.rs                        (new) AgentConfig + availability check; delegates to AcpRuntime
  hermes/
    mod.rs                        (new) same shape
```

### Phase R5 — Feature orchestrator

```
src-tauri/src/
  domain/feature.rs               (new) Feature, FeatureRun, StepExecution, GateDecision, SubtaskRun
  ports/feature_orchestrator.rs   (new) FeatureOrchestrator
  ports/step_executor.rs          (new) StepExecutor, GatePresenter
  ports/notification.rs           (modified) add feature_status_changed, step_progress, gate_required, conflict_detected
  adapters/database/sqlite.rs     (modified) implement FeatureOrchestrator, StepExecutor against SQLite
  adapters/tauri_ui/commands.rs   (modified) feature_start, feature_pause, feature_resume, feature_cancel, feature_get, feature_list, feature_archive, feature_restore, feature_rerun
```

### Phase R6 — Worktree & merge

```
src-tauri/src/
  domain/worktree.rs              (new) SubtaskRun, SubtaskMerge, MergeStrategy
  domain/conflict.rs              (new) ConflictReport, ConflictPolicy
  ports/worktree_mgr.rs           (new) WorktreeManager, MergeExecutor, MrPublisher, ConflictResolver
  adapters/worktree/
    mod.rs                        (new)
    git_ops.rs                    (new) worktree create/remove, branch checkout
    merge.rs                      (new) rebase + merge into feature branch
    conflict.rs                   (new) detect conflicts, surface report
    publish.rs                    (new) open MR via provider instance
```

### Phase R7 — UI

```
src/
  App.tsx                         (rewritten) navigation shell; subscribes to NotificationPort events
  components/
    ProjectRail.tsx               (new) Q24-A
    ProjectHome.tsx               (new) Q21-B
    FeatureDetail.tsx             (new) Q13
    GateView.tsx                  (new) Q13
    WorkflowEditor.tsx            (new) Q19
    WorkflowList.tsx              (new)
    StartFeatureModal.tsx         (new) Q22
    PreFlightPanel.tsx            (new) Q23
    ProviderSettings.tsx          (new) Q17a
    PreferencesScreen.tsx         (new) Q29
    EmptyStateCard.tsx            (new) Q27
    DocsPanel.tsx                 (new) Q27
    ConflictResolver.tsx          (new) Q20 (Monaco 3-way)
    CommandPalette.tsx            (new) Q24 / Q32
    ... (carries: Sidebar, TerminalTabs, SSHTerminal; EnvModal removed in favor of ProviderSettings)

src/docs/                         (new) bundled markdown
  index.md
  first-project.md
  how-workflows-work.md
  connecting-providers.md
  feature-branch-model.md
  conflict-resolution.md
```

---

## 10. Phase Plan (R0–R8)

Each phase has a "Done means…" statement. Phases are sequential; don't start the next until the current is verified. Full breakdown with task checkboxes and verification commands: [`docs/REDESIGN_EXECUTION_PLAN.md`](docs/REDESIGN_EXECUTION_PLAN.md).

### Phase R1 — Greenfield schema & ports

**Scope:** Add the new tables; add the new ports; no UI changes, no agent spawns.

**Done means:**
- `cargo build` passes; `cargo test` passes; the new tables and port contracts are covered.
- The `PricingTable` is hard-coded with the 5–10 most common models.
- The legacy `thread_sessions` table is preserved (for migration safety) but no port surfaces it.

### Phase R4 — Step executor + AcpRuntime

**Scope:** Implement the `StepExecutor` and the ACP client. The runtime is called by the executor, not the UI.

**Done means:**
- A 5-step workflow (research → spec → plan → tasks → implement-stub) runs end-to-end on a local project.
- The `gate` step between plan and tasks actually pauses; the user clicks Approve; the executor resumes.
- A `parallel` step with 3 subtasks runs them; the executor collects all 3 results.
- Every state transition is in `step_executions`; killing and restarting demeteo resumes from the last completed step.

### Phase R5 — Feature orchestrator

**Scope:** The user-facing "Start a feature" flow. Per-feature lifecycle. Re-entry on launch.

**Done means:**
- A user can: open a project → click "New feature" → describe a feature → click "Launch" → see the feature running in ProjectHome → click into FeatureDetail → see the step timeline + telemetry → reach a gate → make a decision → watch the next step run.
- Killing demeteo mid-feature and relaunching surfaces a synthetic gate; the user can resume or restart the interrupted step.

### Phase R6 — Worktree & merge

**Scope:** Per-feature branch, per-subtask worktree, sequential merge, conflict resolution, optional MR.

**Done means:**
- A `parallel` step's subtasks land in `feature/<slug>` via the engine.
- A conflict between two subtasks surfaces at a gate; the user picks auto-agent (spawn resolution) or manual (3-way merge).
- A `publish` step at the end of the workflow opens a draft MR with the right title, body, and source/target branches.

### Phase R7 — UX polish & docs

**Scope:** All the "feel" surfaces. Project rail. Settings. First-run. Docs. Shortcuts.

**Done means:**
- The app is usable end-to-end by a new user with no prior context.
- The state-driven empty card guides the user through provider → project → first feature.
- The sample project runs a real feature on a real public repo, end-to-end, with the full Research → Spec → Plan → Tasks → Implement → Validate loop visible.
- The docs panel has 5+ pages accessible from the "?" icon.
- The command palette fuzzy-finds projects, features, workflows, settings, and actions.

### Phase R8 — Hardening & migration

**Scope:** Schema migration infrastructure. Wipe-and-reinit. Backups. Migration log.

**Done means:**
- The app can ship v1.1 with additive schema changes silently, with no user prompt.
- The app can ship v2.0 with a breaking change; the user is prompted to wipe-and-reinit, with an option to export first.
- A pre-migration backup is always taken; the user can manually restore from `demeteo.db.bak.<timestamp>`.
- The migration log records every migration with timestamp and outcome.

---

## 11. Open Questions (the runtime-relevant subset)

Full list with phase placement: [`docs/REDESIGN_OPEN_QUESTIONS.md`](docs/REDESIGN_OPEN_QUESTIONS.md). The runtime-relevant deferred items:

1. **Second non-ACP runtime** (Anthropic) → v1.1. The runtime trait is transport-neutral; adding a non-ACP adapter is the v1.1 commitment from the legacy design interview.
2. **Per-machine `AgentConfig`** (model, workdir, env) → deferred. Users configure their agents on the host. v1.1 candidate.
3. **WASM provider plugins** → v2+. Third parties shipping provider adapters as WASM modules.
4. **WASM policy plugins** → v2+. The original WASM plugin host from the legacy architecture, deferred with the legacy spec.
5. **Per-step retry policy with planner-as-advisor** → v1.x. The `RetryPolicy` struct on `StepConfig` is reserved but the planner-driven redirect is v1.x.

The full set of deferred items (Q1 multi-feature concurrency, Q19 YAML editor, Q19 save-run-as-template, Q20 deep dry-run, Q21 cost rollup, Q21 smart project home, Q24 tabs/split view, Q8 `command` step type, Q11 telemetry, Q12 auto-update) is in [`docs/REDESIGN_OPEN_QUESTIONS.md`](docs/REDESIGN_OPEN_QUESTIONS.md).
