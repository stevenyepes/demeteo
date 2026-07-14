# Agent Integration Spec (v1, post-pivot, CLI)

This document is the **source of truth for how Demeteo integrates coding
agents** in the multi-agent orchestrator. It captures the runtime trait,
the `CliRuntime` implementation, and the *narrowed* surface that flows
from the pivot: agents are invoked via their CLI as one-shot processes
(`opencode run --format json`, `hermes run --format json`, etc.), not via
ACP JSON-RPC. The `StepExecutor` drives the session (one per step).

The locked decisions are in [`docs/DECISIONS.md`](docs/DECISIONS.md).
The full architecture is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

For the surrounding architecture (hexagonal layout, plugin host, port
trait catalogue), see [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

---

## 1. Scope and Non-Goals (post-pivot)

### v1 ships

- A pluggable `AgentRuntime` trait with one concrete implementation: `UnifiedCliRuntime` (one-shot CLI process + JSON-lines event stream) under `adapters/agent/cli_runtime.rs`.
- Three agent configurations out of the box: **`opencode`**, **`hermes`**, and **`claude-code`** — all via their CLI's JSON-output mode (`opencode run --format json`, `hermes run --format json`, `claude --print --verbose --output-format stream-json`). Each declares the same [capability contract](#41-the-trait) (`display_label`, `lists_models`, `default_model`), so adding a fourth is filling in one descriptor plus `parse_event` / `build_args` / `perm_env` — see [`docs/adapters/CONTRIBUTING-AN-AGENT.md`](docs/adapters/CONTRIBUTING-AN-AGENT.md).
- The runtime serves **both** the planner (an agent session that decomposes the feature into a step DAG) and the subtask agents (sessions that execute a single `agent` or `parallel` step's work). Same trait, same plumbing, different prompts.
- Eager agent session lifecycle scoped to step executions (a process is spawned per `prompt` call, torn down on completion).
- A four-axis `PermissionProfile` (`read_fs | write_fs | execute | network`, each `Allow` or `Deny`) plus a path-shaped `WriteScope` (`None | ArtifactsOnly | All`). Compiled per step from the step's `StepCapability`. Each agent adapter translates the abstract profile to its native dialect at spawn (opencode / hermes → `OPENCODE_PERMISSION` env; claude-code → `--disallowedTools`). `external_directory: "deny"` (opencode) is the worktree scope fence; the chmod fence in `adapters/worktree/git_ops/scope.rs` enforces the artifacts-vs-source path-shape uniformly across every agent.
- Cross-step conversation continuity via per-agent session-id flags so a multi-step workflow shares the agent's context.
- A typed three-layer error model (per-action `ActionError` / per-step `AgentEvent::Error` / per-feature watchdog).
- Per-step checkpoint persistence (DB-backed, populated on every state transition via `StepExecutionPatch`).
- The `AgentEvent` vocabulary is **internal** — consumed by the `StepExecutor`, not by the UI. The UI sees step transitions, not agent transcripts.

### v1 explicitly does NOT include

- **Secret management for the brain's API keys.** The user pre-configures their agent (provider, API key, model) on the host where the agent runs. Demeteo does not read, store, or inject model API keys. **The planner is just another agent session in this respect.** Phase 8+ candidate.
- **A demeteo-side LLM for orchestration.** The "brain" of a *run* is a coding agent (opencode, hermes, or claude-code) invoked by the `StepExecutor`. Demeteo never calls a model provider directly to drive a feature. **Exception — the Memory Agent:** an opt-in, user-configured OpenAI-compatible endpoint (e.g. Ollama) that Demeteo calls directly, in the background, *only* to distill run signals into project memories and to embed them for semantic retrieval. It never drives a feature run; it is disabled by default and its API key lives in the OS keyring. See `adapters/memory_worker.rs` and `adapters/memory_llm.rs`.
- **Resume / context replay across restarts.** A Feature Run is a C-strict opaque cursor; the agent session id is internal. The orchestrator *does* re-enter a feature at the last completed step on launch (synthetic gate on mid-step interrupt), but it does not replay prior agent transcripts to the new session. The step's *artifact* is the cross-restart state.
- **ACP.** The `AcpRuntime`, `JsonRpcClient`, `ToolBridge`, and both transport adapters are deleted in v1. A future `OpencodeServerRuntime` (HTTP client to `opencode serve`) is a v1.1 candidate that would re-introduce a structured protocol for real-time permission approval via the server's `POST /session/:id/permissions/:permissionID` endpoint.
- **Real-time permission approval UX.** The agent enforces permissions via `OPENCODE_PERMISSION`. Demeteo writes the policy at spawn time; the agent enforces it. The gate-step approval surface (user clicks Approve/Reject on the step timeline) is the only real-time human-in-the-loop affordance demeteo provides.
- **Per-agent settings UI (model picker, working dir override, etc.).** The user configures their agent on the host. Demeteo passes `--model` and `--dir` at spawn time; the UI writes the model selection to the DB, which the `StepExecutor` reads at spawn. v1.1 candidate.
- **Auto-restart on transient errors.** Single restart on user request only.
- **Token/cost usage dashboard.** The `Usage` event is wired into the JSON-lines stream but the UI surfaces per-step cost from the `PricingTable`, not a token counter. A v1.x polish item.
- **A chat-style supervisor UI.** The chat UX is gone (per the pivot). The UI is a fleet-control surface; the agent's own chat is not demeteo's concern.
- **Working memory.** No chat, no working memory sidecar. The per-step artifact is the durable record.

### Why CLI is the right bet for v1

The ACP approach (JSON-RPC 2.0 over stdio, capability negotiation, `initialize` / `session/new` / `session/prompt`, tool-call bridging) proved to have five structural failure modes in practice: (1) wire-format drift between agent versions breaking the event mapper, (2) capability-detection hacks for `toolCallUpdate` / `sessionCancel` in two naming conventions, (3) concurrent-call serialization corrupting the JSON-RPC transport when `set_config_option` raced with an in-flight `prompt`, (4) a 5-minute `session/new` timeout with no recovery, and (5) an SSH-process-leak risk when the transport's background reader held an `Arc` past the session's lifetime.

The CLI approach (`opencode run --format json`) sidesteps all five: it is one `Command::spawn` and one stdout pipe with no handshake, no capability negotiation, no session state to leak, and no concurrent calls to serialize. The `opencode serve` HTTP API (v1.1 candidate) would re-introduce a session concept and real-time permission approval for users who need it, without paying the ACP complexity cost in v1.

---

## 2. Locked Decisions (the runtime-relevant ones)

Cross-reference: full table in [`docs/DECISIONS.md`](docs/DECISIONS.md).

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
| 34 | Agent protocol                     | §1 (scope)   |
| 35 | Permission enforcement             | §6           |
| 36 | Cross-step session continuity       | §4.1         |

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

The agent *session* identifier is the CLI `--session <uuid>` argument passed to `opencode run`. It is owned by the `StepExecutor` (in memory) and recorded in `step_executions.agent_session_id` for cross-step continuity within a feature run. It never crosses the Rust↔TypeScript boundary.

**Cross-step continuity.** A multi-step workflow (e.g., research → spec → plan → tasks → implement) shares one agent session id across all `agent` steps within the same feature run, via `--session <uuid> --continue`. Each `parallel` subtask gets its own session id (so subtasks don't pollute each other's context). On feature re-entry after a crash, a fresh session is created; the step's *artifact* is the cross-restart state.

**No resume across restarts at the session level.** A restarted Demeteo finds the `step_executions` row in SQLite; if it was `running`, the orchestrator marks it `interrupted` and surfaces a synthetic gate (see §3.5). The next directive (i.e., the user clicking "Resume" or the orchestrator continuing) creates a fresh agent session for the step. The step's *artifact* is the cross-restart state.

### 3.2 The planner is just an agent session

There's no special "planner port" or "planner runtime." The planner is a coding agent session (opencode, hermes, or claude-code) invoked with a *planning prompt* — the same `CliRuntime`, the same CLI invocation, the same JSON-lines event stream. The only special thing is the prompt template, which lives in the workflow step's config (the first `agent` step in the starter pack's Research → Spec → Plan → Tasks → Implement → Validate workflow, for example).

The planner's selection is per-project
(`ProjectSettings::default_agent_kind` + `default_model` — see
[`docs/DDD_MODEL.md`](docs/DDD_MODEL.md) §2 Project Management and
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §2 `ProjectRepository`).
The user picks the planner when configuring project settings, and may
layer a per-workflow override (`ProjectWorkflowOverride` with
`step_id = None`) and per-step overrides. The orchestrator resolves
the effective agent at feature-start time via
`resolve_execution_context`.

### 3.3 Project host + provider instance

Each project has exactly one host (`Project.host: { type: "local" | "remote", ... }`). A project is bound to a provider instance at creation; the instance's PAT is used for both `git clone` (via `SshRepositoryCloner` / `LocalFsRepositoryCloner`) and `mr_publish` (via `MrPublisher`). The provider instance is keyed by `(kind, host)` to support multiple GitLab instances and GitHub Enterprise Server.

The agent runs on the **same host as the worktree**:
- `auth_type == "local"` → `tokio::process::Command` with the user's shell env inherited. `CliRuntime::start` resolves the binary from `PATH` and spawns directly.
- `auth_type in {"key", "password", "agent"}` → SSH channel via `ExecutionPort::spawn_interactive`. Demeteo connects over the existing authenticated SSH session, runs the agent binary over a long-lived `ssh2::Channel`, and owns both ends of the stdio.

**No per-machine override.** The location is implied by the project's host. One less way to misconfigure.

### 3.4 The `AgentEvent` vocabulary is internal

`src-tauri/src/domain/agent_event.rs`:

```rust
use serde::{Deserialize, Serialize};
use crate::domain::action::ActionKind;
use crate::domain::artifact::Artifact;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streamed assistant text delta. The frontend appends to the most recent text block.
    Text { delta: String },

    /// A durable artifact was produced (a file the agent just wrote,
    /// a derived diff, a worktree navigation pointer, etc.). The
    /// `StepExecutor` collects these into a per-step buffer and
    /// resolves them against the step's declared `ArtifactDecl`s at
    /// `TurnComplete`. **This is the cross-restart durable record** —
    /// text events are ephemeral UI signals, this is what survives.
    ArtifactProduced { artifact: Artifact },

    /// Agent wants to do something. The `tool_call_id` is the agent's id; the
    /// `intercept_id` is Demeteo's internal handle (always minted for traceability).
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

    /// Agent publishes an execution plan (opencode plan mode, etc.)
    Plan { entries: Vec<PlanEntry> },

    /// Token / cost telemetry. Emitted standalone by opencode and hermes
    /// (multiple times per turn); attached to `TurnComplete.usage` by
    /// Claude Code (one final snapshot per turn).
    Usage(Usage),

    /// Soft error from the agent
    Error {
        code: String,
        message: String,
        recoverable: bool,
    },

    /// Agent finished the turn. The channel closes after this.
    /// `usage` carries the terminal cumulative token/cost snapshot for
    /// the turn when the agent's wire format bundles them onto the
    /// result line (Claude Code). Parsers that emit usage as separate
    /// `Usage` events leave this `None`.
    TurnComplete {
        stop_reason: StopReason,
        usage: Option<Usage>,
    },

    /// Agent switched modes (e.g., plan -> build). Carries the new mode id.
    ModeChanged { mode_id: String },

    /// Agent updated a config option (model, mode, reasoning level, etc.)
    ConfigChanged { config_id: String, value: String },
}

/// Token / cost snapshot.
///
/// A standalone struct (rather than an inline enum variant) so that the
/// `TurnComplete { usage: Option<Usage> }` carrier can hold the same
/// shape as the standalone `Usage` event without a self-referential
/// enum. `cache_read_input_tokens` and `cache_creation_input_tokens`
/// are emitted by Claude Code today; opencode and hermes emit `0` until
/// their wire formats expose them. The shared
/// `UsageAccumulator` treats all four numeric fields as monotonically
/// cumulative per turn.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_usd: Option<f64>,
    pub cache_read_input_tokens: u64,
    pub cache_creation_input_tokens: u64,
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
pub struct PlanEntry {
    pub step: String,
    pub status: String, // "pending" | "in_progress" | "done" | "blocked"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndOfTurn,
    Cancelled,
    MaxTokens,
    Error,
}
```

The `Text`, `ArtifactProduced`, and `Plan` events are consumed by the `StepExecutor` to build the step's artifact (per the type-driven defaults in [`docs/DECISIONS.md`](docs/DECISIONS.md) decision 28). The `Usage` event feeds the per-step `cost_usd` and the per-feature telemetry (per the locked decision 15). The `TurnComplete` event drives the step's state transition. `ModeChanged` and `ConfigChanged` flow back to the `AgentTerminalDrawer` for interactive sessions.

The UI does **not** consume the agent event stream. It consumes `feature_status_changed` / `step_progress` / `gate_required` / `conflict_detected` events from the `NotificationPort`.

### 3.5 Step status state machine

`StepExecution.status` values:

| Value             | Meaning                                              | Set by                                |
|-------------------|------------------------------------------------------|---------------------------------------|
| `pending`         | Step is next up, not yet started                     | `StepExecutor` on run start           |
| `running`         | Agent session is active                              | `StepExecutor` on first AgentEvent    |
| `verifying`       | Step finished its agent turn; verifier / QA hook running | `StepExecutor` between turn and gate |
| `awaiting_gate`   | A gate is awaiting user decision                     | `StepExecutor` on `gate_required`     |
| `completed`       | Step finished; artifact written                      | `StepExecutor` on `TurnComplete`      |
| `failed`          | Step failed; user action required                    | `StepExecutor` on terminal Error      |
| `skipped`         | Step was skipped (e.g., conflict resolution skip)    | `StepExecutor` on user skip           |
| `interrupted`     | App was killed mid-step; synthetic gate on re-entry  | `StepExecutor` on shutdown watchdog   |

Per-step checkpoints (decision 14) are atomic: a step transitions to `completed` only when its artifact is written and (if it's a gate) its `gate_decision` is recorded. Mid-step crashes surface as `interrupted`, and the next launch offers a synthetic gate with "Resume" (re-run the step) or "Skip" options.

The four "blocking" statuses for the predecessor-running guard
(`StepExecutor::step_retry`, `GatePresenter::gate_decide`) are
`pending | running | verifying | awaiting_gate`. `completed`,
`failed`, `interrupted`, and `skipped` are non-blocking — the guard
walks `steps_for_feature(target.feature_id)` and returns
`Err(AppError::validation)` on the first non-terminal predecessor with
`step_index < target.step_index`.

### 3.6 The worktree-of-record is `feature/<slug>`

Subtask worktrees branch off `feature/<slug>` (decision 16). The orchestrator creates `feature/<slug>` off the project's canonical branch at feature start. Each subtask's worktree is branched off the *latest* `feature/<slug>` (i.e., after any prior subtask merges). Subtask branches merge into `feature/<slug>` in topological DAG order via the `MergeExecutor`.

The worktree scope is enforced via the agent's own `external_directory: "deny"` permission rule (rendered by `PermissionPolicyPort::render_for` into the `OPENCODE_PERMISSION` env var). The `PermissionPolicy` struct maps to the JSON shape `{"external_directory": "deny", "edit": "allow", ...}`. The user's `feature/<slug>` branch is touched only at merge time, never by an agent directly.

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

/// The capabilities Demeteo asks of a coding agent, declared once per runtime
/// instead of being inferred from `match kind { ... }` string lists. Adding an
/// agent means filling this in — no downstream site special-cases the kind.
pub struct AgentCapabilities {
    pub display_label: &'static str,       // human-facing name for the UI
    pub lists_models: bool,                // exposes a `<binary> models` subcommand
    pub default_model: Option<&'static str>, // seeds cost fallback; None if dynamic
}

pub trait AgentRuntime: Send + Sync {
    fn kind(&self) -> &'static str;        // equals AgentKind::as_str

    fn capabilities(&self) -> AgentCapabilities;

    fn binary(&self) -> &'static str;      // defaults to kind()

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

The `AgentContext` holds the resolved binary, CLI args, env vars (e.g. `OPENCODE_PERMISSION` for the opencode-family agents), the model, and the worktree `cwd`. Every supported agent is a one-shot CLI runtime that takes its model via a `--model` flag built from `AgentContext.model`; there is no `OPENCODE_CONFIG_CONTENT`/ACP model path. The `AgentRegistry` is simplified: each call to `CliRuntime::start` spawns a fresh `Command::spawn` and returns an `Arc<dyn AgentSession>`; there is no session deduplication or reuse across step executions. The `StepExecutor` holds the `Arc<AgentSession>` for the duration of one `prompt` call.

### 4.2 Lifecycle: one-shot per prompt call, scoped to step execution

A `StepExecution` row exists in SQLite before any agent does. The agent process is spawned at the moment the executor calls `session.prompt(text)`, and torn down on:

- **Step completion** (terminal `TurnComplete` or terminal `Error`): process exits, stdout drain completes.
- **Step failure** (terminal `Error`): process exits, working state preserved.
- **Step retry** (per Q14 retry policy): the previous process is killed; a new one is spawned on the next `prompt` call.
- **Feature pause / cancel / re-run**: the `StepExecutor` holds the `Arc<AgentSession>`; calling `session.cancel()` kills the child process.
- **App shutdown**: the `StepExecutor`'s `Arc<AgentSession>` is dropped; `AgentSession::kill()` is called in the `Drop` impl, ensuring the process is reaped.

There is no session registry or deduplication. The `AgentContext` carries `--session <uuid>` (for cross-step continuity) and `--continue` (to append to the same conversation). The `Arc<AgentSession>` lives for the duration of one `prompt` call.

### 4.3 Where the process lives

The agent process runs on the **same host as the worktree** (the project's host, per §3.3):

- `auth_type == "local"` → `tokio::process::Command::new(binary)` with the user's shell env inherited. `CliRuntime` resolves the binary from `PATH` and spawns directly.
- `auth_type in {"key", "password", "agent"}` → SSH channel via `ExecutionPort::spawn_interactive`. Demeteo connects over the existing authenticated SSH session, runs the agent binary over a long-lived `ssh2::Channel`, and owns both ends of the stdio.

**No per-machine override.** The location is implied by the project's host. One less way to misconfigure.

### 4.4 The CLI event stream

`CliRuntime` produces an `AgentEvent` stream by spawning `opencode run --format json [args...]`, draining `stdout` line-by-line, and passing each nd-JSON line through a per-agent `parse_event` function:

```rust
pub type EventParser = fn(line: &str) -> Option<AgentEvent>;

pub struct CliAgentRuntime {
    pub kind_str: &'static str,
    pub binary: &'static str,
    pub extra_args: &'static [&'static str],
    pub install_cmd: &'static str,
    pub parse_event: EventParser,
}
```

The `parse_event` function is registered per agent kind and maps the agent's JSON-line shape onto `AgentEvent` variants (`Text`, `Usage`, `TurnComplete`, `Error`). Unknown event types are silently dropped so future agent versions don't break the stream.

The `AgentTransport` trait and its two implementations (`LocalSubprocessTransport`, `RemoteSshTransport`) are deleted. The `JsonRpcClient` is deleted. The `AcpRuntime` is deleted.

### 4.5 Conflict resolution cascade (decision 20)

When a `parallel` step's subtask merge conflicts with the `feature/<slug>` branch, the `MergeExecutor` produces a `ConflictReport` and the `ConflictResolver` cascade kicks in:

1. **Auto-agent** (default `conflict_policy: "auto_agent"`): spawn a conflict-resolution subtask — a fresh agent session with a constrained prompt ("resolve the conflicts in these N files; do not modify unrelated code; produce a resolution commit"). Cost-capped (default 2 attempts, $0.50).
2. **Manual** (on auto-agent failure or `conflict_policy: "auto_human"`): open the `ConflictResolver` UI (Monaco 3-way merge).
3. **Skip / Abort** (always available): mark the subtask `skipped` or the feature `aborted`.

The cascade is enforced by the `StepExecutor` and `ConflictPolicy` (per-project setting). See [`docs/OPEN_QUESTIONS.md`](docs/OPEN_QUESTIONS.md) §1 for the deferred per-step retry policy.

---

## 5. The CliRuntime

### 5.1 Why CLI

The CLI approach (one-shot `opencode run --format json`) sidesteps five structural failure modes that plagued the ACP approach: (1) wire-format drift between agent versions breaking the event mapper, (2) capability-detection hacks for `toolCallUpdate` / `sessionCancel` in two naming conventions, (3) concurrent-call serialization corrupting the JSON-RPC transport when `set_config_option` raced with an in-flight `prompt`, (4) a 5-minute `session/new` timeout with no recovery, and (5) an SSH-process-leak risk when the JSON-RPC transport's background reader held an `Arc` past the session's lifetime.

The CLI approach is: one `Command::spawn`, one stdout pipe, no handshake, no capability negotiation, no session state to leak, no concurrent calls to serialize. The `opencode serve` HTTP API (v1.1 candidate) would re-introduce a session concept and real-time permission approval via `POST /session/:id/permissions/:permissionID`.

### 5.2 The wire format

Each agent emits nd-JSON on stdout when run with the JSON-output flag:

| Agent        | CLI invocation                                           | Event shape                                      |
|--------------|---------------------------------------------------------|-------------------------------------------------|
| opencode     | `opencode run --format json [args...]`                 | `{"type":"text","part":{"text":"..."}}`, `{"type":"step_finish","part":{"reason":"stop","tokens":{...},"cost":...}}`, `{"update":{"sessionUpdate":"agent_message_chunk",...}}` |
| hermes       | `hermes run --format json [args...]`                    | `{"kind":"text","delta":"..."}`, `{"kind":"usage","inputTokens":...,"outputTokens":...,"costUsd":...,"cacheReadInputTokens":...,"cacheCreationInputTokens":...}`, `{"kind":"end_turn"}` |
| claude-code  | `claude --print --verbose --output-format stream-json [args...]`  | `{"type":"system","subtype":"init","session_id":"..."}`, `{"type":"assistant","message":{"content":[{"type":"text","text":"..."},{"type":"tool_use","id":"...","name":"Bash","input":{...}}]}}`, `{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"...","content":"...","is_error":false}]},"tool_use_result":{...}}`, `{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.187,"usage":{"input_tokens":...,"output_tokens":...,"cache_creation_input_tokens":...,"cache_read_input_tokens":...}}` |

The per-agent parser is registered on `UnifiedCliRuntime.parse_event` and
maps the agent's wire format onto `AgentEvent` variants. Unknown event
types are silently dropped so future agent versions don't break the
stream.

<!-- EXAMPLE: new -->

### 5.3 Cross-step session continuity

A multi-step feature run uses one session id across all `agent` steps:

```
# Step 1 (planner): spawn with a new session id
opencode run --format json --session <uuid-1> --title "plan" "<prompt>"

# Step 2 (implement): continue the same session
opencode run --format json --session <uuid-1> --continue "<prompt>"

# Step 3 (validate): same session
opencode run --format json --session <uuid-1> --continue "<prompt>"
```

Parallel subtasks each get their own session id so subtask sessions don't pollute each other's context. Planner and workers have separate session ids.

**Tier 1 implementation (token optimization):**

The orchestrator's `AgentRegistry` keys sessions by `f_id` for the
main feature agent. The `StepExecutor` reuses the live `AgentSession`
across all `agent` steps in a feature — `get_or_spawn` returns the
existing session when one is registered, and only spawns a fresh
process on the very first turn (or after a watchdog reset, see
below). The captured session id from the first `system` init event
is threaded into `--session <id> --continue` (opencode) /
`--resume <id>` (claude-code, hermes) on every subsequent prompt. This unlocks vendor
prompt-cache hits on the static prefix (system prompt + tool
definitions) across steps, materially reducing token usage on
multi-step features.

**Context-window watchdog:**

`AgentSession::cumulative_tokens` is implemented by the CLI runtime
and tracked per-session (input + output monotonic max from
`Usage` / `TurnComplete.usage` events). The driver's
`maybe_watchdog_reset` runs after each completed step and compares
the session's cumulative tokens against the model's context-window
budget (resolved via `PricingTable::context_window`). When usage
exceeds 80% of the budget, the driver kills the session, sets
`session_dirty = true`, builds a one-shot recap from the last
completed step's artifact (`session_resume_summary`), and the next
step's `spawn_agent_session` spawns a fresh session that injects
the recap at the top of its prompt.

**Dead-session fallback:**

If the underlying agent process dies between steps (network blip,
crash), `spawn_agent_session` detects via `AgentSession::is_alive()`
and falls back to `registry.kill(f_id) + get_or_spawn` rather than
attempting `--continue` against a dead session id.

### 5.4 Install flow

When the `StepExecutor` needs to spawn a step, it calls `runtime.is_available(exec, machine_id)`:

```
step_executor.spawn_step(step_execution_id, agent_kind)
  → runtime.is_available(exec, machine_id) ?
       yes → runtime.start(ctx)
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

**Lazy after first failure.** If the user later uninstalls the agent, the spawn fails with ENOENT and re-triggers the install flow.

**No user-editable install commands.** The command is static per adapter, baked into the source. The user can only consent or cancel.

### 5.5 Permission policy per spawn

Each `UnifiedCliRuntime::start` call compiles the abstract
`PermissionProfile` to the agent's native dialect and injects it as an
env var or CLI flag:

opencode / hermes (`OPENCODE_PERMISSION` env, complete policy — never `ask`):

```
OPENCODE_PERMISSION={"edit":"allow","read":"allow","bash":"allow","webfetch":"deny","websearch":"deny","external_directory":"deny","doom_loop":"allow"}
```

claude-code (`--disallowedTools`):

```
claude --print --verbose --output-format stream-json \
       --dangerously-skip-permissions \
       --disallowedTools "WebSearch,WebFetch" \
       --exclude-dynamic-system-prompt-sections \
       --setting-sources user,project \
       --strict-mcp-config
```

The compiled policy comes from `AgentContext::permissions`
(`ports/agent_runtime.rs:65-73` — see `opencode_permission_json`).
The four axes (`read_fs`, `write_fs`, `execute`, `network`) are each
`Allow` or `Deny` only — never `ask`. The artifacts-vs-source path
distinction is enforced uniformly by the OS-level chmod fence in
`adapters/worktree/git_ops/scope.rs`, driven by
`StepCapability::write_scope`. `external_directory: "deny"` (opencode)
is the worktree scope fence; the binary refuses to operate on paths
outside `cwd`. claude-code has no equivalent tool-level setting, so the
chmod fence is the only enforcement on that agent.

There is **no** `bash: "ask"` path in the compiled policy. A denied
tool is rejected instantly; the agent gets a tool-result error and
keeps going. Nothing blocks waiting on a human — that's the
autonomous-pipeline guarantee. The gate-step approval surface (user
clicks Approve / Redirect on the step timeline) is the only
real-time human-in-the-loop affordance Demeteo provides.

When the `OPENCODE_PERMISSION` env var is absent (e.g., direct CLI
invocation outside demeteo), the agent applies its own default policy.

### 5.5.1 Claude Code auth: let Claude own it

**Demeteo handles no Anthropic credentials.** Claude Code resolves and refreshes its own credential (OAuth from the macOS keychain, or `~/.claude/.credentials.json` on any OS) exactly as it does for a normal `claude` invocation. There is no token extraction, no `settings.json` write, no keychain shell-out, and no per-spawn env injection on our side.

This works because pipeline steps **do not** pass `--bare`. Per `claude --help`, `--bare` sets `CLAUDE_CODE_SIMPLE=1`, which "skips … keychain reads" and makes "Anthropic auth … strictly `ANTHROPIC_API_KEY` or `apiKeyHelper` via `--settings` (OAuth and keychain are never read)." That single flag is what previously forced OAuth users into `Not logged in · Please run /login` and pushed us toward extracting and injecting the token ourselves.

We adopted `--bare` only for its prompt-cache benefit (a byte-identical static system-prompt prefix across worktrees). We get that benefit from narrower flags that leave native auth intact (`build_claude_args`, emitted only when `ctx.bare_mode = true`):

- `--exclude-dynamic-system-prompt-sections` — moves per-machine sections (cwd, env info, git status, memory paths) out of the cached prefix into the first user message.
- `--setting-sources user,project` — loads user- and project-level config (the skills, `CLAUDE.md`, and settings the user committed to the repo) but **not** machine-local `settings.local.json`. Project config is identical across a feature's worktrees (same repo at the same commit), so including it is cache-neutral; only `local` varies per checkout, which is what we drop.
- `--strict-mcp-config` — ignores project MCP servers (we pass none), keeping the tool set identical across worktrees.

**Cross-OS coverage** is whatever the `claude` CLI itself supports — macOS keychain, Linux/Windows credential file — because we defer entirely to it. A user who exports `ANTHROPIC_API_KEY` (or `ANTHROPIC_AUTH_TOKEN`) in their shell is inherited by the spawned child and still honored.

**Token refresh is correct by construction.** Short-lived OAuth access tokens are renewed by Claude's own refresh-token exchange; because we never copy the access token out of the keychain, there is no stale-credential-on-disk to go bad and no global `~/.claude/settings.json` mutation to contaminate the user's own interactive `claude`.

### 5.5.2 Why not extract the token (`--settings`, `apiKeyHelper`, or `settings.json` env)?

Earlier iterations wired `--settings <path>` + `apiKeyHelper`, then `settings.json`-env injection. Both are kept here as rejected alternatives:

1. **`apiKeyHelper` can't carry OAuth.** Claude sends `apiKeyHelper` output as the `x-api-key` header; Anthropic rejects OAuth tokens (`sk-ant-oat01-*`) there with HTTP 401. Only `Authorization: Bearer` (via `ANTHROPIC_AUTH_TOKEN`) works for OAuth.
2. **`/bin/sh` splits the helper path on whitespace.** Claude invokes the helper via `/bin/sh -c '<path>'`, so the macOS app data dir under `~/Library/Application Support/<bundle>/` fails with `exited 127`.
3. **Extracting the token at all is the deeper mistake.** OAuth access tokens expire and only Claude's refresh exchange renews them. Any copy we make (into `settings.json` env or a child env var) eventually goes stale, and writing the user's *global* `settings.json` also changes the behavior of their own `claude` sessions. Not extracting the token sidesteps every one of these failure modes.

The trade-off: without `--bare`, user- and project-level hooks/skills/memory load during autonomous steps. That's intentional for skills/`CLAUDE.md` — the user committed those to the repo to shape how agents work on it — but it also means project hooks fire. Only machine-local `settings.local.json` is excluded (via `--setting-sources user,project`). Verify prompt-cache reuse across worktrees holds by comparing `cache_read_input_tokens` in the stream-json `usage` across two worktree runs.

### 5.6 Adapters

Three agents, all via `UnifiedCliRuntime`:

- `adapters/agent/opencode/mod.rs` — `{ kind_str: "opencode", binary: "opencode", install_cmd: "curl -fsSL https://opencode.ai/install | bash", perm_env: opencode_permission_env, … }`.
- `adapters/agent/hermes/mod.rs` — `{ kind_str: "hermes", binary: "hermes", install_cmd: "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash", perm_env: opencode_permission_env, … }`.
- `adapters/agent/claude_code/mod.rs` — `{ kind_str: "claude-code", binary: "claude", install_cmd: "npm install -g @anthropic-ai/claude-code", perm_env: no_permission_env (claude-code enforces via `--disallowedTools`), … }`.

The `UnifiedCliRuntime` is generic over the binary — the agent-specific logic is just the declared `AgentCapabilities` (`display_label` / `lists_models` / `default_model` / `effort_levels`), the availability check (which binary name to look up), the `parse_event` function, the `build_args` function, the `perm_env` translator (`opencode_permission_env` or `no_permission_env`), and the `effort_env` translator. See [`docs/adapters/CONTRIBUTING-AN-AGENT.md`](docs/adapters/CONTRIBUTING-AN-AGENT.md) for the full walkthrough of adding one.

#### The `effort_levels` capability

`AgentCapabilities.effort_levels: &'static [EffortLevel]` declares which levels of the canonical ladder (`low` < `medium` < `high` < `xhigh` < `max`) an agent accepts **per invocation**. It is part of the capability contract, not a UI nicety: `list_agents` ships it to the frontend, which uses it to populate every effort picker, and `EffortLevel::clamp_for` uses the same table to project a requested level onto what the agent can actually take *before* argv is built. So the UI cannot offer an unsupported level, and if it somehow did, the adapter still could not emit one.

| Agent | `effort_levels` | Carried on the wire as |
|---|---|---|
| `claude-code` | all five | `--effort <v>` **and** `CLAUDE_CODE_EFFORT_LEVEL=<v>` (the env var outranks the flag, so it must be set explicitly — a developer with it exported would otherwise silently override every run) |
| `codex` | `low, medium, high, xhigh` | `-c model_reasoning_effort=<v>` |
| `opencode` | all five | `--variant <v>` |
| `hermes` | `&[]` (none) | nothing — see below |

An **empty** list is a first-class answer, not a gap: hermes exposes effort only through `agent.reasoning_effort` in `$HERMES_HOME/config.yaml` and has no per-invocation control, so it declares nothing, emits nothing, and the frontend greys its effort control out with a tooltip. Neither codex nor opencode *validates* an effort it doesn't know (codex wraps an unknown value as `Custom(String)` and sends it; opencode treats an unsupported `--variant` as a silent no-op), which is precisely why Demeteo owns the clamp rather than trusting the CLI to reject a bad value.

### 5.7 Disclaimer

Both the README and the `ProviderSettings` UI strings should clarify that Demeteo's `opencode` integration targets `anomalyco/opencode` (the open-source coding agent project) and that Demeteo is **not affiliated with the opencode project**. The `anomalyco/opencode` README explicitly asks projects using "opencode" in their name to make this clear.

---

## 6. The Scope Fence

### 6.1 The problem

Without a fence, an agent running with `cwd = <worktree>` can `cd ..` out of the worktree, read `/etc/passwd`, write to `~/.ssh/authorized_keys`, etc. The four-axis `PermissionProfile` (§5.5) catches writes and bash via tool-level rules, but tool names aren't a *path* check — a `Read` tool pointed at `/etc/passwd` still slips through unless the binary itself enforces a worktree scope.

### 6.2 The implementation

The scope fence has three layers:

1. **Tool-level.** opencode / hermes use the
   `OPENCODE_PERMISSION` env var (§5.5) for tool-level allow / deny.
   claude-code uses `--disallowedTools` with the same intent. The
   `external_directory: "deny"` rule (opencode only) is the worktree
   scope fence: paths outside `cwd` are refused at the binary level.
2. **Path-shaped.** The OS-level chmod fence in
   `adapters/worktree/git_ops/scope.rs` sets `chmod a-w` on
   source-tree paths before each step and restores it after. The
   fencings come from `WriteScope::{None, ArtifactsOnly, All}`, which
   `StepCapability::write_scope` derives. The artifacts-vs-source
   distinction is a *path* shape that no agent's tool model can
   express, so the fence enforces it uniformly across every agent.
3. **Pre-exec.** `spawn_interactive` (§2 S3 in
   [`docs/RELIABILITY_PLAN.md`](docs/RELIABILITY_PLAN.md)) probes the
   cwd with `test -d <cwd> && echo OK`, verifies the binary resolves
   on the remote `$PATH` with `command -v <binary>`, and drains
   stderr for the first 200 ms post-exec. Fail-fast surfaces as
   `AgentStartError::SpawnFailed("cwd not found: …")` or
   `AgentStartError::SpawnFailed("binary not found: …")`.

### 6.3 What this does NOT solve

- **Tool-name → path leak on claude-code.** claude-code has no
  `external_directory` setting, so the chmod fence is the only
  enforcement. The orchestrator restores the source tree's write
  permissions at step boundaries, but a step that uses a non-source
  write path (`/tmp/`, `node_modules/`, `target/`) is still
  user-visible; project owners compensate with
  `WorktreeStrategy::extra_writable_paths`.
- **TOCTOU.** Symlink races between resolution and execution are
  best-effort.
- **Cross-worktree access in the feature branch.** The scope fence
  protects the subtask's worktree. A subtask that *should* see prior
  subtask merges gets them via the merge into `feature/<slug>` (not
  via a separate read path).

---

## 7. Error Model

### 7.1 Per-action errors (typed)

`ActionError` (`ports/agent_execution.rs:24-29`):

```rust
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionError {
    Network { message: String },
    NotFound { message: String },
    Internal { message: String },
}
```

The frontend maps each variant to a small set of recovery chips. The
typed envelope is returned from `AgentExecutionPort::submit_agent`;
`submit` (the legacy hand-rolled-action path) still returns
`Result<CommandOutcome, String>` for backward compatibility. Existing
free-form `Err(String)` returns on other commands are migrated
incrementally — the rule is "every new error path returns `ActionError`."

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

The full list of commands is in [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) §4 and registered in [`src-tauri/src/lib.rs:461-585`](src-tauri/src/lib.rs). The runtime-relevant commands (the ones the `StepExecutor` and the agent lifecycle handler invoke on the agent runtime) live under `commands::agent_lifecycle`:

```rust
#[tauri::command]
async fn agent_start(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
    machine_id: String,
    agent_kind: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_install_and_start(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
    machine_id: String,
    agent_kind: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_prompt(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
    text: String,
) -> Result<(), String>;  // prompt is enqueued; events flow via NotificationPort

#[tauri::command]
async fn agent_cancel(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_restart(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_get_session_info(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
) -> Result<SessionInfo, String>;

#[tauri::command]
async fn agent_set_mode(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
    mode_id: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_set_config_option(
    state: tauri::State<'_, AppContext>,
    thread_id: String,
    config_id: String,
    value: String,
) -> Result<(), String>;
```

The interactive `AgentTerminalDrawer` calls `agent_start` /
`agent_prompt` / `agent_cancel` / `agent_set_mode` /
`agent_set_config_option`; pipeline steps don't use this command set
(the `StepExecutor` spawns sessions directly through `AgentRegistry`).

The legacy `agent_prompt` returned a `Channel<AgentEvent>` for per-turn UI streaming. In the post-pivot design, the prompt is enqueued on the `AgentRegistry`'s internal channel; the UI never subscribes to per-turn streams. The `StepExecutor` consumes the `AgentEvent` stream and emits `step_progress` / `feature_status_changed` / `gate_required` events that the UI does subscribe to.

---

## 9. File Layout (post-pivot, post-refactor)

The files touched by the agent integration work, organized by phase. Existing files modified are marked with `(modified)`; new files are marked with `(new)`. Paths reflect the current backend refactor layout (Phases A–G of `docs/BACKEND_REFACTOR_TASKS.md`).

### Phase R1 — Port + domain skeleton

```
src-tauri/src/domain/
  agent_event.rs                  (modified) AgentEvent vocabulary (Text / ToolCall / ToolCallUpdate / Plan / Usage / Error / TurnComplete / ModeChanged / ConfigChanged / ArtifactProduced)
  models/                         (split) per bounded context: agent_config.rs, feature.rs, machine.rs, merge.rs, notification.rs, project.rs, provider.rs, thread.rs, timeouts.rs, workflow.rs
  permission.rs                   (new) StepCapability, PermissionProfile, WriteScope, Access, resolve_profile

src-tauri/src/ports/
  agent_runtime.rs                (modified) AgentContext gains permissions, bare_mode; add opencode_permission_env, no_permission_env
  db.rs                           (split) eight sub-ports (MachineRepository, ThreadRepository, ProjectRepository, FeatureRepository, WorkflowRepository, GateRepository, AppSettingsRepository, MergeAuditRepository, NotificationRepository) + Patch value objects
```

### Phase R4 — Step executor + `UnifiedCliRuntime`

```
src-tauri/src/adapters/agent/
  mod.rs                          (modified) remove acp submodule; add claude_code submodule
  registry.rs                     (simplified) remove session dedup; Arc<AgentSession> per prompt call
  cli_runtime.rs                  (modified) inject OPENCODE_PERMISSION env var; thread session id; --session/--continue/--resume flags per agent; declared AgentCapabilities
  opencode/
    mod.rs                        (modified) return UnifiedCliRuntime; add parse_opencode_event
  hermes/
    mod.rs                        (modified) return UnifiedCliRuntime; add parse_hermes_event; --resume <sid> for cross-step continuity
  claude_code/
    mod.rs                        (new) UnifiedCliRuntime + parse_claude_event + disallowed_tools_for; --disallowedTools / --exclude-dynamic-system-prompt-sections / --setting-sources user,project / --strict-mcp-config
  event_stream/
    mod.rs
    turn.rs                       (new) stream_agent_turn — typed TurnResult { Success(TurnOutcome), Interrupted, Failed(String) }
    cleanup.rs
```

### Phase R5 — Step executor + feature orchestrator

```
src-tauri/src/
  domain/models/feature.rs        (new) Feature, StepExecution, SubtaskRun, GateDecision, StepOverride
  ports/step_executor.rs          (new) StepExecutor + GatePresenter + SyncOutcomeView (all async)
  ports/notification.rs           (modified) slimmed; per-step events only
  adapters/step_executor/
    mod.rs
    driver.rs                     (new) ExecutionDriver — re-entry, watchdog, terminal status
    driver/
      failure.rs                  (new) fail_step_and_feature, StepOutcome::{Goto, Loop, Stop}
      verifier.rs                 (new) max_iterations, on_failure -> goto
    driver_registry.rs
    gate_waiter.rs
    steps/
      agent/
        mod.rs, spawn.rs, artifacts.rs, error_message.rs
      gate.rs
      parallel/
        mod.rs, planner.rs, subtask.rs, list_unmerged.rs
      sync.rs
    artifacts/, impl_traits/, setup.rs, sync.rs, tests/, updates.rs
  adapters/database/              (split) eight sub-port impls + merge audit + notification tables
  adapters/tauri_ui/              commands.rs + events.rs
```

### Phase R6 — Worktree, merge, sync, scope

```
src-tauri/src/
  domain/models/merge.rs          (new) SubtaskMerge, FeatureSync, MergeOutcome, ConflictReport, ConflictFile, ConflictPolicy, UpstreamSyncOutcome/Failure
  ports/worktree_ops.rs           (new) WorktreeOpsPort + MergePreCheck::{AlreadyMerged, CleanMerge, WouldConflict}
  ports/merge.rs                  (new) MergePort
  ports/conflict.rs               (new) ConflictPort
  ports/mr_publisher.rs           (new) MrPublisher
  adapters/worktree/
    mod.rs
    git_ops/
      mod.rs, clone.rs, strategy.rs, worktree.rs, merge.rs, sync.rs, health.rs, scope.rs, tests.rs
  adapters/conflict.rs
  adapters/merge.rs
  adapters/mr_monitor.rs
  adapters/mr_publisher.rs
```

### Phase R7 — UI

```
src/
  App.tsx                         (rewritten) navigation shell; subscribes to NotificationPort events
  components/
    ProjectRail.tsx               (new) Q24-A
    ProjectHome.tsx               (new) Q21-B
    FeatureDetail.tsx             (new) Q13 — step timeline + predecessor-running guard
    GateView.tsx                  (new) Q13 — full-screen takeover + predecessor-running guard
    WorkflowEditor.tsx            (new) Q19 — form-first
    WorkflowList.tsx              (new)
    StartFeatureModal.tsx         (new) Q22
    PreFlightPanel.tsx            (new) Q23
    NewProjectView.tsx            (new) — slim modal
    ProviderSettings.tsx          (new) Q17a
    PreferencesScreen.tsx         (new) Q29
    MemoryAgentSettings.tsx       (new) — Memory Agent config (under Preferences → Memory)
    MachinesView.tsx              (new) — per-host agent profiles
    EmptyStateCard.tsx            (new) Q27
    DocsPanel.tsx                 (new) Q27 — bundled markdown viewer
    CommandPalette.tsx            (new) Q24 / Q32
    NotificationBell.tsx          (new)
    AgentTerminalDrawer.tsx       (new) — interactive sessions, agent_set_mode / agent_set_config_option
    AttachmentDropzone.tsx, AttachmentChip.tsx (new)
    ArtifactViewer.tsx, CodeEditorView.tsx (new)
    ... (carries: Sidebar, TopBar, ProjectSettings; EnvModal replaced by ProviderSettings)

src/docs/                         (new) bundled user-facing help markdown
  index.md, first-project.md, how-workflows-work.md,
  connecting-providers.md, feature-branch-model.md,
  conflict-resolution.md, keyboard-shortcuts.md, troubleshooting.md
```

---

## 10. Phase Plan (R0–R8)

> **Status note (2026):** Phases R1–R7 have shipped. The remaining
> follow-ups are tracked in [`docs/OPEN_QUESTIONS.md`](docs/OPEN_QUESTIONS.md)
> (Q1 multi-feature concurrency, Q2 YAML editor, Q3 save-run-as-template,
> Q4 deep dry-run, Q8 command step, Q12 auto-update, Q14 second non-CLI
> runtime, etc.).

Each phase has a "Done means…" statement. Phases were sequential; this
section is retained for traceability.

### Phase R1 — Schema & ports (shipped)

**Scope:** Add the new tables; add the new ports; no UI changes, no agent spawns.

**Done means:**
- `cargo build` passes; `cargo test` passes; the new tables and port contracts are covered.
- The `PricingTable` is hard-coded with the 5–10 most common models.
- The `DatabasePort` is split into eight sub-ports.

### Phase R4 — Step executor + `UnifiedCliRuntime` (shipped)

**Scope:** Implement the `StepExecutor` and the `UnifiedCliRuntime`. The runtime is called by the executor, not the UI.

**Done means:**
- A 5-step workflow (research → spec → plan → tasks → implement-stub) runs end-to-end on a local project.
- The `gate` step between plan and tasks actually pauses; the user clicks Approve; the executor resumes.
- A `parallel` step with 3 subtasks runs them; the executor collects all 3 results.
- Every state transition is in `step_executions`; killing and restarting demeteo resumes from the last completed step.
- ACP / JSON-RPC transport adapters are deleted from the codebase.

### Phase R5 — Feature orchestrator (shipped)

**Scope:** The user-facing "Start a feature" flow. Per-feature lifecycle. Re-entry on launch.

**Done means:**
- A user can: open a project → click "New feature" → describe a feature → click "Launch" → see the feature running in ProjectHome → click into FeatureDetail → see the step timeline + telemetry → reach a gate → make a decision → watch the next step run.
- Killing demeteo mid-feature and relaunching surfaces a synthetic gate; the user can resume or restart the interrupted step.

### Phase R6 — Worktree, merge, sync (shipped)

**Scope:** Per-feature branch, per-subtask worktree, sequential merge, conflict resolution, optional MR.

**Done means:**
- A `parallel` step's subtasks land in `feature/<slug>` via the engine.
- A conflict between two subtasks surfaces at a gate; the user picks auto-agent (`feature_resolve_sync_conflicts` spawns a resolution agent and revalidates the step) or manual (`GateView` re-render with the file list).
- A `publish` step at the end of the workflow opens a draft MR with the right title, body, and source/target branches.
- `feature_sync` syncs `feature/<slug>` against `origin/<default>` and returns a typed `SyncOutcomeView::{Ok, Conflict, Resolved, ResolutionFailed}`.

### Phase R7 — UX polish & docs (shipped)

**Scope:** All the "feel" surfaces. Project rail. Settings. First-run. Docs. Shortcuts.

**Done means:**
- The app is usable end-to-end by a new user with no prior context.
- The state-driven empty card guides the user through provider → project → first feature.
- The sample project runs a real feature on a real public repo, end-to-end, with the full Research → Spec → Plan → Tasks → Implement → Validate loop visible.
- The docs panel has 5+ pages accessible from the "?" icon.
- The command palette fuzzy-finds projects, features, workflows, settings, and actions.
- The predecessor-running guard surfaces in both `FeatureDetail` (Retry Step button) and `GateView` (Approve / Redirect buttons).

### Phase R8 — Hardening & migration (shipped; ongoing additive migrations)

**Scope:** Schema migration infrastructure. Wipe-and-reinit. Backups. Migration log.

**Done means:**
- The app can ship v1.x with additive schema changes silently, with no user prompt.
- The app can ship a breaking change; the user is prompted to wipe-and-reinit, with an option to export first.
- A pre-migration backup is always taken; the user can manually restore from `demeteo.db.bak.<timestamp>`.
- The migration log records every migration with timestamp and outcome.

---

## 11. Open Questions (the runtime-relevant subset)

Full list with phase placement: [`docs/OPEN_QUESTIONS.md`](docs/OPEN_QUESTIONS.md). The runtime-relevant deferred items:

1. **Second non-CLI runtime** (e.g. `opencode serve` HTTP, or a raw Anthropic API for a custom planner) → v1.1. The runtime trait surface (`ports/agent_runtime.rs`) already supports this; the per-adapter `perm_env` + `build_args` are the only knobs.
2. **Per-machine structured `AgentConfig`** (model, workdir, env, pricing override) → v1.x. The legacy shell / custom-http `AgentProfile` rows exist; a first-class structured config is deferred.
3. **WASM provider plugins** → v2+. Third parties shipping provider adapters as WASM modules.
4. **WASM policy plugins** → v2+. WASM plugin host loaded from `~/.config/demeteo/plugins/`.
5. **Per-step retry policy with planner-as-advisor** → v1.x. `on_failure -> goto` + `max_iterations` are in place; a planner-as-advisor redirect (the agent drafts the redirect target) is a v1.x candidate.

The full set of deferred items (Q1 multi-feature concurrency, Q19 YAML editor, Q19 save-run-as-template, Q20 deep dry-run, Q21 cost rollup, Q21 smart project home, Q24 tabs/split view, Q8 `command` step type, Q11 telemetry, Q12 auto-update) is in [`docs/OPEN_QUESTIONS.md`](docs/OPEN_QUESTIONS.md).
