# Agent Integration Spec (v1, post-pivot, CLI)

This document is the **source of truth for how Demeteo integrates coding
agents** in the multi-agent orchestrator. It captures the runtime trait,
the `CliRuntime` implementation, and the *narrowed* surface that flows
from the pivot: agents are invoked via their CLI as one-shot processes
(`opencode run --format json`, `hermes run --format json`, etc.), not via
ACP JSON-RPC. The `StepExecutor` drives the session (one per step).

The pivot's locked decisions are in [`docs/REDESIGN_DECISIONS.md`](docs/REDESIGN_DECISIONS.md).
The full architecture is in [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md).
The master plan is in [`REDESIGN_PLAN.md`](REDESIGN_PLAN.md).

For the surrounding architecture (hexagonal layout, plugin host, port
trait catalogue), see [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md).

---

## 1. Scope and Non-Goals (post-pivot)

### v1 ships

- A pluggable `AgentRuntime` trait with one concrete implementation: `CliRuntime` (one-shot CLI process + JSON-lines event stream).
- Four agent configurations out of the box: **`opencode`**, **`hermes`**, **`claude-code`**, and **`antigravity`** — all via their CLI's JSON-output mode (`opencode run --format json`, `hermes run --format json`, `claude --print --output-format stream-json`, `agy --print -`).
- The runtime serves **both** the planner (an agent session that decomposes the feature into a step DAG) and the subtask agents (sessions that execute a single `agent` or `parallel` step's work). Same trait, same plumbing, different prompts.
- Eager agent session lifecycle scoped to step executions (a process is spawned per `prompt` call, torn down on completion).
- `OPENCODE_PERMISSION` env var per spawn; `external_directory: "deny"` to scope the worktree. The `PermissionPolicyPort` renders the policy JSON.
- Cross-step conversation continuity via `--session <uuid> --continue` flags so a multi-step workflow shares the agent's context.
- A typed three-layer error model (per-action / per-step / per-feature).
- Per-step checkpoint persistence (DB-backed, populated on every state transition).
- The `AgentEvent` vocabulary is **internal** — consumed by the `StepExecutor`, not by the UI. The UI sees step transitions, not agent transcripts.

### v1 explicitly does NOT include

- **Secret management for the brain's API keys.** The user pre-configures their agent (provider, API key, model) on the host where the agent runs. Demeteo does not read, store, or inject model API keys. **The planner is just another agent session in this respect.** Phase 8+ candidate.
- **A demeteo-side LLM.** The "brain" is a coding agent (opencode, hermes, claude-code, or antigravity) invoked by the `StepExecutor`. Demeteo never calls a model provider directly.
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

There's no special "planner port" or "planner runtime." The planner is a coding agent session (opencode, hermes, claude-code, or antigravity) invoked with a *planning prompt* — the same `CliRuntime`, the same CLI invocation, the same JSON-lines event stream. The only special thing is the prompt template, which lives in the workflow step's config (the first `agent` step in the starter pack's Research → Spec → Plan → Tasks → Implement → Validate workflow, for example).

The planner's selection is per-project (`Project.planner: { machine_id, agent_kind }` — see `ProjectRepository` in [`docs/REDESIGN_ARCHITECTURE.md`](docs/REDESIGN_ARCHITECTURE.md) §2). The user picks the planner when creating the project. The orchestrator resolves it at feature-start time.

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

The `AgentContext` holds the resolved binary, CLI args, env vars (including `OPENCODE_PERMISSION` and `OPENCODE_CONFIG_CONTENT`), and the worktree `cwd`. The `AgentRegistry` is simplified: each call to `CliRuntime::start` spawns a fresh `Command::spawn` and returns an `Arc<dyn AgentSession>`; there is no session deduplication or reuse across step executions. The `StepExecutor` holds the `Arc<AgentSession>` for the duration of one `prompt` call.

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

The cascade is enforced by the `StepExecutor` and `ConflictPolicy` (per-project setting). See [`docs/REDESIGN_OPEN_QUESTIONS.md`](docs/REDESIGN_OPEN_QUESTIONS.md) §1 for the deferred per-step retry policy.

---

## 5. The CliRuntime

### 5.1 Why CLI

The CLI approach (one-shot `opencode run --format json`) sidesteps five structural failure modes that plagued the ACP approach: (1) wire-format drift between agent versions breaking the event mapper, (2) capability-detection hacks for `toolCallUpdate` / `sessionCancel` in two naming conventions, (3) concurrent-call serialization corrupting the JSON-RPC transport when `set_config_option` raced with an in-flight `prompt`, (4) a 5-minute `session/new` timeout with no recovery, and (5) an SSH-process-leak risk when the JSON-RPC transport's background reader held an `Arc` past the session's lifetime.

The CLI approach is: one `Command::spawn`, one stdout pipe, no handshake, no capability negotiation, no session state to leak, no concurrent calls to serialize. The `opencode serve` HTTP API (v1.1 candidate) would re-introduce a session concept and real-time permission approval via `POST /session/:id/permissions/:permissionID`.

### 5.2 The wire format

Each agent emits nd-JSON on stdout when run with the JSON-output flag:

| Agent        | CLI invocation                                           | Event shape                                      |
|--------------|---------------------------------------------------------|-------------------------------------------------|
| opencode     | `opencode run --format json [args...]`                 | `{"type":"text","content":"..."}`               |
| hermes       | `hermes run --format json [args...]`                    | (same as opencode; confirm at implementation)    |
| claude-code  | `claude --print --output-format stream-json [args...]`  | `{"type":"text","content":"..."}`               |
| antigravity  | `agy --print - [args...]`                              | `{"type":"text_delta","data":{"text":"..."}}`  |

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

Each `CliRuntime::start` call injects the spawn-time policy as an env var:

```
OPENCODE_PERMISSION={"edit":"allow","read":"allow","bash":"ask","webfetch":"deny","websearch":"deny","external_directory":"deny","doom_loop":"ask"}
```

The policy object is resolved from `AppContext.permission_policy` and written to `AgentContext.permission_policy_json` before spawn. The binary receives it as `OPENCODE_PERMISSION` in its env. The worktree scope is enforced by `external_directory: "deny"` (the binary refuses to operate on paths outside `cwd`).

`bash: "ask"` is the only gate that requires real-time human-in-the-loop. When the agent emits a bash action, the step pauses at the gate and the user approves or rejects via the `GateView` UI before the agent receives the result.

When the `OPENCODE_PERMISSION` env var is absent (e.g., direct CLI invocation outside demeteo), the agent applies its own default policy.

### 5.6 Adapters

Four agents, all via `CliRuntime`:

- `adapters/agent/opencode/mod.rs` — wraps `CliRuntime` with config `{ binary: "opencode", args: [], env: {}, install_command: "curl -fsSL https://opencode.ai/install | bash" }`.
- `adapters/agent/hermes/mod.rs` — wraps `CliRuntime` with config `{ binary: "hermes", args: [], env: {}, install_command: "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash" }`.
- `adapters/agent/claude_code/mod.rs` — wraps `CliRuntime` with config `{ binary: "claude", args: ["--print", "--output-format", "stream-json"], env: {}, install_command: "npm install -g @anthropic-ai/claude-code" }`.
- `adapters/agent/antigravity/mod.rs` — wraps `CliRuntime` with config `{ binary: "agy", args: ["--print", "-"], env: {}, install_command: "curl -fsSL https://raw.githubusercontent.com/everestVentures/antigravity/main/install.sh | bash" }`.

The `CliRuntime` is generic over the binary — the agent-specific logic is just the `AgentConfig`, the availability check (which binary name to look up), and the `parse_event` function.

### 5.7 Disclaimer

Both the README and the `ProviderSettings` UI strings should clarify that Demeteo's `opencode` integration targets `anomalyco/opencode` (the open-source coding agent project) and that Demeteo is **not affiliated with the opencode project**. The `anomalyco/opencode` README explicitly asks projects using "opencode" in their name to make this clear.

---

## 6. The Scope Fence

### 6.1 The problem

Without a fence, an agent running with `cwd = <worktree>` can `cd ..` out of the worktree, read `/etc/passwd`, write to `~/.ssh/authorized_keys`, etc. The policy decorator catches writes and bash, but the bash-prefix match isn't a *path* check — `cat /etc/passwd` slips through unless the user has a `Reject` rule for `cat /etc/*`.

### 6.2 The implementation

The scope fence is `external_directory: "deny"` in the `OPENCODE_PERMISSION` env var (§5.5). The binary itself enforces this — paths outside `cwd` are refused at the binary level. No `PolicyEngine` involvement, no path canonicalization, no pre-rule.

Bash commands that escape the worktree are caught by `bash: "ask"` — the user is prompted at the gate and can reject. There is no path-level bash prefix check.

### 6.3 What this does NOT solve

- **Bash command scope.** `cd / && cat /etc/passwd` is subject to `bash: "ask"`, not a path fence. The user must be attentive at gate prompts.
- **TOCTOU.** Symlink races between resolution and execution are best-effort.
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

### Phase R4 — Step executor + CliRuntime

```
src-tauri/src/adapters/agent/
  mod.rs                          (modified) remove acp submodule; add claude_code, antigravity submodules
  registry.rs                     (simplified) remove session dedup; Arc<AgentSession> per prompt call
  opencode/
    mod.rs                        (modified) return CliAgentRuntime; add parse_opencode_event
  hermes/
    mod.rs                        (modified) return CliAgentRuntime; add parse_hermes_event
  claude_code/
    mod.rs                        (new) CliAgentRuntime + parse_claude_code_event
  antigravity/
    mod.rs                        (new) CliAgentRuntime + parse_antigravity_event
  cli_runtime.rs                  (modified) inject OPENCODE_PERMISSION env var; add --session --continue wiring
  permission_policy.rs            (new) PermissionPolicyPort + WorktreeScopedPolicy
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

### Phase R4 — Step executor + CliRuntime

**Scope:** Implement the `StepExecutor` and the `CliRuntime`. The runtime is called by the executor, not the UI.

**Done means:**
- A 5-step workflow (research → spec → plan → tasks → implement-stub) runs end-to-end on a local project.
- The `gate` step between plan and tasks actually pauses; the user clicks Approve; the executor resumes.
- A `parallel` step with 3 subtasks runs them; the executor collects all 3 results.
- Every state transition is in `step_executions`; killing and restarting demeteo resumes from the last completed step.
- `AcpRuntime`, `jsonrpc.rs`, `event_mapper.rs`, `tool_bridge.rs`, `transport_local.rs`, `transport_ssh.rs` are deleted from the codebase.

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
