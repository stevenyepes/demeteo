# Agent Integration Spec (v1)

This document is the **source of truth** for how Demeteo integrates coding agents. It captures every locked design decision from the v1 design interview, the data model, the port surface, the Tauri command surface, the UI behavior, and the phase plan. **If something here conflicts with code or other docs, this document wins** for v1 scope; flag the conflict and update the others.

For the surrounding architecture (hexagonal layout, plugin host, port trait catalogue), see **[ARCHITECTURE.md](file:///home/jsteven/Projects/demeteo/ARCHITECTURE.md)**.

---

## 1. Scope and Non-Goals

### v1 ships

- A pluggable `AgentRuntime` trait with one concrete implementation: `AcpRuntime` (JSON-RPC over stdio, per the [Agent Client Protocol](https://agentclientprotocol.com/) v1).
- Two agent configurations out of the box: **`hermes`** and **`opencode`** (the `anomalyco/opencode` project, the open-source coding agent — *not* the archived `opencode-ai/opencode`).
- Lazy agent session lifecycle bound to thread lifecycle.
- A scope fence in the policy engine that auto-rejects any file path resolving outside the thread's worktree.
- A typed three-layer error model (per-action / per-turn / per-session).
- Persistent per-thread working memory (DB-backed, populated by `Read` tool calls, with file metadata).
- Auto-Inspector behavior on the first `Read` of a turn.

### v1 explicitly does NOT include

- **Secret management.** The user pre-configures their agent (provider, API key, model) on the host where the agent runs. Demeteo does not read, store, or inject model API keys. Phase 8 candidate.
- **Resume / context replay across restarts.** A Demeteo thread is a C-strict opaque cursor; the agent session id is internal. Phase 8 candidate (persisted transcript + replay).
- **Non-ACP transports.** The `AcpRuntime` is the only runtime in v1. Adding HTTP/Server / stdio JSON-lines / MCP-as-agent transports is Phase 7e and explicitly chosen to be a *different* protocol so the abstraction gets a real test.
- **WASM policy plugins.** Designed in `ARCHITECTURE.md`; deferred. The policy decorator + scope fence cover v1 approval needs.
- **Per-agent settings UI (model picker, working dir override, etc.).** The picker shows "Default settings" + a "Configure…" stub. Full settings UI is Phase 7d.
- **Auto-restart on transient errors.** Single restart on user request only.
- **Token/cost usage dashboard.** The `Usage` event is wired into the protocol stack but the UI indicator is stubbed in v1.

### Why "ACP only" is the right bet for v1

In mid-2026 the two leading open-source coding agents (Hermes and anomalyco/opencode) both ship **ACP as a first-class, documented surface** in their main navigation. ACP is JSON-RPC 2.0 over stdio (local) or Streamable HTTP / WebSocket (remote), with stable v1 wire format, official SDKs for Rust/Python/TS/Java/Kotlin, and an active RFD process. The bet is not "will ACP be adopted" — it's "every serious agent in this category speaks ACP or will have to."

We design the runtime trait to be transport-neutral so we can add non-ACP runtimes later, but the v1 surface area is intentionally narrow: one trait, one implementation, two configs. This keeps the dispatcher, the policy decorator, the channels, and the UI ignorant of which agent is in use. The "second adapter must be non-ACP" rule from the design interview is a Phase 7e commitment to validate the abstraction, not a v1 requirement.

---

## 2. Locked Design Decisions

The 17 decisions from the v1 design interview, in order. Each has a one-line summary and a pointer to the section of this doc that implements it.

| # | Decision | Section |
|---|---|---|
| 1 | Agent session lazy on first directive, idle-timeout teardown | §4 |
| 2 | Thread is the only UI-visible key; agent session id is internal | §3.1 |
| 3 | C-strict: no resume, no transcript persistence | §3.1 |
| 4 | Per-turn `Channel<AgentEvent>` for content; global Tauri events for side effects | §6 |
| 5 | `AcpRuntime` only; two agents: `hermes`, `opencode` | §5 |
| 6 | Agent runs on same host as worktree (local→local, server→SSH) | §5.2 |
| 7 | Official install scripts only, explicit consent, eager on first thread per machine | §5.3 |
| 8 | Scope fence pre-rule in policy engine, overridable by explicit user rules | §7 |
| 9 | No secret management; user pre-configures agent | §1 (non-goal) |
| 10 | Stacked agent card in `NewThreadModal`, auto-default, blocks if none | §8.1 |
| 11 | `Machine.agents` migrates to `{kind, enabled}[]`; `ThreadSession` gets `agent_kind` | §8.2 |
| 12 | Frontend optimistic status, backend confirms; adds `spawning / installing / error` | §8.3 |
| 13 | Stop replaces Send during running; implicit cancel+redirect on Enter; idempotent cancel; drains pending intercepts | §8.4 |
| 14 | Minimal info event on turn complete; refocus input; no summary card | §8.5 |
| 15 | Three-layer error model with typed variants and recovery affordances | §9 |
| 16 | New `thread_working_memory` table; cap 20; cleared on restart; metadata via SFTP | §10 |
| 17 | Inspector auto-opens on first Read (or after 5s+ gap); updates on subsequent Reads; never on writes; yes on re-reads-after-writes | §8.6 |

---

## 3. Domain Model

### 3.1 Thread is the only key

`ThreadSession` (`src-tauri/src/domain/models.rs:56`) gains one new field:

```rust
pub struct ThreadSession {
    pub id: String,
    pub machine_id: String,
    pub title: String,
    pub mode: String,               // 'worktree', 'adhoc'
    pub branch: Option<String>,
    pub repo_path: Option<String>,
    pub sandbox_path: Option<String>,
    pub status: String,             // extended: see §8.3
    pub agent_kind: Option<String>, // NEW: "opencode" | "hermes" | None
}
```

The agent *session* identifier (ACP's `sessionId`) is owned by the `AgentRegistry` and never crosses the Rust↔TypeScript boundary. From the UI's perspective, a thread *is* the session. The ACP `sessionId` is allowed to change (auto-compact, reconnect) and if the UI held it, we'd have a synchronization nightmare. If we ever need to display it, surface it as a *display string* that may auto-update.

**No resume across restarts.** A restarted Demeteo app finds the thread row in SQLite, but the agent subprocess is gone. The next directive creates a fresh agent session. The thread is "remembered as a place" but the agent starts from scratch on every (re)launch.

### 3.2 `AgentEvent` vocabulary

`src-tauri/src/domain/agent_event.rs` (NEW):

```rust
use serde::{Deserialize, Serialize};
use crate::domain::policy::ActionKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    /// Streamed assistant text delta. The frontend appends to the most recent text block.
    Text { delta: String },

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

    /// Token / cost telemetry
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: Option<f64>,
    },

    /// Soft error from the agent
    Error {
        code: String,        // "rate_limit" | "auth_failed" | "context_overflow" | "internal" | "cancelled"
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
pub struct PlanEntry {
    pub step: String,
    pub status: String, // "pending" | "in_progress" | "done" | "blocked"
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

### 3.3 `InterceptPayload` extension

`src-tauri/src/domain/intercept.rs:6`:

```rust
pub struct InterceptPayload {
    pub intercept_id: String,
    pub thread_id: String,
    pub machine_id: String,
    pub action: ActionKind,
    pub target: String,
    pub preview: Option<String>,
    pub created_at: String,
    pub tool_call_id: Option<String>, // NEW: Some(...) for agent-originated; None for hand-rolled
}
```

The `tool_call_id` is opaque to the UI. It's used by `tool_bridge.rs` to correlate ACP `tool_call/update` messages with in-flight `PendingIntercept` entries, and to scope the result returned to the agent.

### 3.4 Policy rejection shaped as a tool result

`src-tauri/src/domain/intercept.rs:55`:

```rust
pub enum Resolution {
    Approve,
    Reject { feedback: String },
    // NEW: explicit "this is a tool-call-shaped failure, not a bash output"
    RejectAsToolFailure { feedback: String },
}
```

`PolicyEnforcedExecutionPort::submit` is updated so that, when the action originated from an agent tool call (the `tool_call_id` is set), rejections are returned to the agent as a structured `tool_call/update { status: Failed, content: [{type: "text", text: feedback}] }` rather than as a synthetic bash output. The existing bash-shaped path is preserved for legacy / hand-rolled `request_action` calls.

---

## 4. Runtime Trait and Lifecycle

### 4.1 The trait

`src-tauri/src/ports/agent_runtime.rs` (NEW):

```rust
use std::pin::Pin;
use std::collections::HashMap;
use std::sync::Arc;
use tokio_stream::Stream;
use serde::{Deserialize, Serialize};

use crate::domain::agent_event::AgentEvent;

#[derive(Debug, Clone)]
pub struct AgentContext {
    pub thread_id: String,
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
    /// Stable identifier; matches `AgentConfig.kind`
    fn kind(&self) -> &'static str;

    /// Check if the binary is reachable (which / command -v on the target host)
    fn is_available(&self, machine_id: &str) -> bool;

    /// The official install command, shown verbatim in the consent prompt
    fn install_command(&self) -> &'static str;

    /// Spawn the agent and return a session handle. The session is fully
    /// initialized (capability negotiation, session/new, etc.) before this returns.
    fn start(&self, ctx: AgentContext) -> Result<Arc<dyn AgentSession>, AgentStartError>;
}

pub trait AgentSession: Send + Sync {
    /// The runtime's own session id; never escapes the backend
    fn session_id(&self) -> &str;

    /// Submit a directive. The returned stream yields `AgentEvent`s until
    /// `TurnComplete` (or terminal `Error`) is emitted, at which point the
    /// stream closes.
    fn prompt(&self, text: &str) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;

    /// Cancel the in-flight turn. Idempotent: no-op if turn is already done.
    fn cancel(&self) -> Result<(), String>;
}
```

### 4.2 Lifecycle: lazy on first directive

A `ThreadSession` row exists in SQLite before any agent does. The agent session is created at the moment the user submits the first directive for that thread, and torn down on:

- **Idle timeout**: configurable, default 5 minutes after last `TurnComplete`.
- **Thread delete**: clean kill, drain pending intercepts.
- **Thread restart** (per §9.3): clean kill, working memory cleared.
- **App shutdown**: clean kill, drain pending intercepts.

The `AgentRegistry` (in `adapters/agent/registry.rs`) holds the `HashMap<ThreadId, Arc<AgentSession>>` and is the only thing that knows which threads have live agents. When a thread is deleted or the app shuts down, the registry iterates and calls `kill()` on each session.

### 4.3 Where the process lives

The agent process runs on the **same host as the worktree**:

- `Machine.auth_type == "local"` → `tokio::process::Command` with the user's shell env inherited.
- `Machine.auth_type in {"key", "password", "agent"}` → SSH channel via the new `ExecutionPort::spawn_interactive` method. Demeteo connects over the existing authenticated SSH session, runs the agent binary over a long-lived `ssh2::Channel`, and Demeteo owns both ends of the stdio.

**No per-machine override.** The location is implied by the machine's auth type. One less way to misconfigure.

### 4.4 The transport layer

The `AgentRuntime` operates on `bytes-in, bytes-out`. The transport layer that produces those bytes is a separate concern, factored out as `AgentTransport` so the runtime is transport-blind:

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

`AcpRuntime` takes an `AgentTransport` and a `NotificationPort` (for emitting the global intercept/result events). It produces an `AgentEvent` stream by parsing the transport's stdout as JSON-RPC and mapping ACP methods onto `AgentEvent` variants.

---

## 5. The AcpRuntime

### 5.1 Why ACP

[Agent Client Protocol](https://agentclientprotocol.com/) v1 is the standard protocol for agent↔client communication. It's JSON-RPC 2.0 over stdio (local) or Streamable HTTP / WebSocket (remote). v1 wire format is stable. Official SDKs exist for Rust (`agent-client-protocol` crate), Python, TypeScript, Kotlin, Java.

Both target agents in v1 speak ACP as a first-class feature:
- **Hermes** has a dedicated `acp_adapter/` module and `acp_registry/` in its repo.
- **anomalyco/opencode** lists **ACP Support** in the main navigation of its docs site (`opencode.ai/docs/acp/`).

### 5.2 Method surface (v1 subset)

We implement the *client* side (Demeteo is the ACP client, the agent is the ACP server). v1 needs:

| Method | Direction | Maps to |
|---|---|---|
| `initialize` | client → agent | one-time capability negotiation |
| `authenticate` | client → agent | only if agent advertises `authMethods` |
| `session/new` | client → agent | creates the per-turn session; returns `sessionId` |
| `session/prompt` | client → agent | sends user directive; response is a stream of `session/update` notifications |
| `session/cancel` | client → agent | stops in-flight turn |
| `session/set_mode` | client → agent | optional: switch build/plan modes |
| `fs/read_text_file` | agent → client | delegated to `PolicyEnforcedExecutionPort::read_file` |
| `fs/write_text_file` | agent → client | delegated to `PolicyEnforcedExecutionPort::write_file` |
| `terminal/create` | agent → client | delegated to `ExecutionPort::run_command` (returns handle) |
| `terminal/output` | agent → client | streamed |
| `terminal/wait_for_exit` | agent → client | blocks |
| `terminal/release` | agent → client | cleanup |

Agent-initiated notifications we listen for:

| Notification | Maps to |
|---|---|
| `session/update` (text chunk) | `AgentEvent::Text` |
| `session/update` (tool_call) | `AgentEvent::ToolCall` |
| `tool_call/update` | `AgentEvent::ToolCallUpdate` |
| `session/update` (plan) | `AgentEvent::Plan` |
| `session/usage_update` | `AgentEvent::Usage` |
| end of stream | `AgentEvent::TurnComplete` |

### 5.3 Install flow

The `EnvModal` lets the user toggle `enabled` for each `AgentConfig`. When the user clicks "Launch Thread" with an agent selected, the dispatcher checks availability:

```
agent_start(thread_id, agent_kind)
  → registry.spawn(kind, ctx)
  → runtime.is_available(machine_id) ?
       yes → spawn
       no  → return AgentStartError::NotFound(binary_name) + install_command
```

On `NotFound`, the frontend shows a consent modal with the install command shown verbatim:

> **Install opencode on `spectacular`?**
> The following official script will be run via SSH:
> ```
> curl -fsSL https://opencode.ai/install | bash
> ```
> [Cancel] [Install and continue]

On consent, the frontend invokes `agent_install_and_start(thread_id, agent_kind)` which:

1. Runs the install command over the appropriate transport (local shell or SSH).
2. Re-checks availability.
3. If available, spawns the agent and returns the session handle.
4. If still not found after install, returns an error and the thread is left in `error` state.

**Eager on first thread per machine, lazy after.** The result of the availability check is cached per `(machine_id, agent_kind)` for the duration of the app session. If the user later uninstalls the agent, the lazy fallback (the spawn itself fails with ENOENT) re-triggers the install flow.

**No user-editable install commands.** The command is static per adapter, baked into the source. The user can only consent or cancel.

### 5.4 The tool bridge

When the agent sends `fs/read_text_file { path }`, the runtime doesn't have direct filesystem access. It calls into the `tool_bridge.rs` module, which:

1. Resolves `path` against the thread's `sandbox_path` (absolute path, no `..`, symlinks resolved via the existing `SftpEntry` metadata).
2. Calls `PolicyEnforcedExecutionPort::submit` with `AgentAction::Read { path: resolved }` — same path as the UI's `Test Intercept` button.
3. The policy engine runs the existing rules **plus the scope fence pre-rule** (see §7).
4. If approved (or auto-approved by rule), the file is read via the underlying `ExecutionPort` and the result is returned to the agent.
5. If rejected (by rule, by scope fence, or by user), a `tool_call/update { status: Failed, content: [{type: "text", text: reason}] }` is sent to the agent.

This means every file operation the agent attempts is subject to *the same policy and scope checks* as a hand-rolled `request_action` call. The agent has no back door.

### 5.5 Adapters (v1)

Two agents, both via `AcpRuntime`:

- `adapters/agent/opencode/mod.rs` — wraps `AcpRuntime` with config `{ binary: "opencode", args: ["acp"], env: {}, install_command: "curl -fsSL https://opencode.ai/install | bash" }`.
- `adapters/agent/hermes/mod.rs` — wraps `AcpRuntime` with config `{ binary: "hermes", args: ["acp"], env: {}, install_command: "curl -fsSL https://hermes-agent.nousresearch.com/install.sh | bash" }`.

The `AcpRuntime` itself is generic over the binary — the agent-specific logic is just the `AgentConfig` and the availability check (which binary name to look up).

### 5.6 Disclaimer

Both the README and the `EnvModal` UI strings should clarify that Demeteo's `opencode` integration targets `anomalyco/opencode` (the open-source coding agent project) and that Demeteo is **not affiliated with the opencode project**. The `anomalyco/opencode` README explicitly asks projects using "opencode" in their name to make this clear.

---

## 6. Streaming: Per-Turn Channels

### 6.1 Why per-turn channels

Multiple threads can be active concurrently. A global Tauri event bus would interleave events from different turns; the UI would have to filter by `thread_id` and accept best-effort ordering. Per-turn channels give us:

- Scoped event stream per turn (no global multiplexing).
- No frontend-side filtering.
- Clean lifetime: the channel closes on `TurnComplete` or terminal `Error`, the React `for await` loop exits, the UI flips to `idle`.

The codebase already uses `Channel<T>` for streaming (SSH terminal data via `sessionRegistry.ts:8-11`), so this extends an existing pattern, not a new one.

### 6.2 Tauri command shape

```rust
#[tauri::command]
async fn agent_prompt(
    state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
    text: String,
) -> Result<Channel<AgentEvent>, String> {
    let registry = state.registry.clone();
    let session = registry.get_or_spawn(&thread_id).await?;
    let (tx, rx) = tokio::sync::mpsc::channel::<AgentEvent>(64);
    let stream = session.prompt(&text);

    tokio::spawn(async move {
        tokio::pin!(stream);
        while let Some(event) = stream.next().await {
            if tx.send(event).await.is_err() {
                break; // frontend dropped the channel; turn is dead
            }
        }
    });

    Channel::from(rx) // Tauri Channel wraps the receiver
}
```

### 6.3 Global events stay

The existing `permission_requested` and `command_executed` Tauri events are unchanged. They are *side effects* of the turn, conceptually outside the per-turn stream. The supervisor plane already listens to them globally. Adding a third event, `thread_status_changed`, lets the backend correct the frontend's optimistic status.

### 6.4 Frontend handler

`src/agentSessionRegistry.ts` (NEW) holds the per-thread session metadata. `App.tsx` wires `sendDirective`:

```typescript
const sendDirective = async (threadId: string) => {
  if (!supervisorInput.trim()) return;
  const text = supervisorInput;
  setSupervisorInput("");

  if (threads.find(t => t.id === threadId)?.status === "running") {
    // Implicit cancel + redirect
    await invoke("agent_cancel", { threadId });
  }

  const channel = new Channel<AgentEvent>();
  channel.onmessage = (event) => handleAgentEvent(threadId, event);
  await invoke("agent_prompt", { threadId, text, tauriChannel: channel });
};
```

`handleAgentEvent` dispatches on `event.kind`:
- `text` → append to current text block in `streams[threadId]`.
- `tool_call` → emit intercept card; route through `request_action` which now respects the `tool_call_id`.
- `tool_call_update` → update the in-flight intercept card.
- `plan` → render as a special block in the stream.
- `usage` → update the (stub) sidebar indicator.
- `error` → render as a `directive`-style event with error styling; if not recoverable, disable input.
- `turn_complete` → append the completion info event; flip status to `idle`; refocus input.

---

## 7. The Scope Fence

### 7.1 The problem

Without a fence, an agent running with `cwd = sandbox_path` can `cd ..` out of the worktree, read `/etc/passwd`, write to `~/.ssh/authorized_keys`, etc. The policy decorator catches writes and bash, but the bash-prefix match isn't a *path* check — `cat /etc/passwd` slips through unless the user has a `Reject` rule for `cat /etc/*`.

The scope fence is a *path-only* pre-rule that runs before any user rule.

### 7.2 The implementation

Extend `src-tauri/src/domain/policy_engine.rs` with a pre-rule check. The pre-rule is constructed at policy-compile time from the thread's `sandbox_path` and is evaluated *first* on every `AgentAction`:

```rust
pub struct ScopeFence {
    pub sandbox_path: PathBuf,
}

impl ScopeFence {
    pub fn check(&self, action: &AgentAction) -> Option<PolicyDecision> {
        let target_path = match action {
            AgentAction::Read { path }
            | AgentAction::Edit { path, .. }
            | AgentAction::Write { path, .. } => path,
            AgentAction::RunBash { .. } => return None, // bash handled by prefix policy
        };

        let resolved = match resolve_path(&self.sandbox_path, target_path) {
            Ok(p) => p,
            Err(_) => return Some(PolicyDecision::Reject {
                reason: "path resolution failed".into(),
            }),
        };

        if !resolved.starts_with(&self.sandbox_path) {
            return Some(PolicyDecision::Reject {
                reason: format!("path '{}' is outside thread scope", target_path),
            });
        }

        None // inside scope; defer to user rules
    }
}
```

`PolicyEngine::evaluate` is updated to call `ScopeFence::check` first. A returned `Some(decision)` short-circuits the user rules. Bash actions return `None` from `check`, so the scope fence is *invisible* for bash and the existing prefix policy still applies.

### 7.3 Path resolution

`resolve_path(sandbox, target)`:
- If `target` is absolute, canonicalize it (resolving symlinks via the existing SFTP `get_metadata`).
- If `target` is relative, join with `sandbox`, then canonicalize.
- Return `Err` if the path doesn't exist *or* if canonicalization fails (we want to fail closed).

For `Edit` and `Write` actions, the target is the *destination*, not a path being read. The fence applies to the destination the same way.

### 7.4 Overriding the fence

The fence returns `Reject`, not `EscalateToUser`. So a user rule with `Approve` for a specific outside-scope path *won't* override it — the fence is evaluated first and short-circuits.

The escape hatch for power users: a new `source` on `PolicyRule` (already in the schema at `domain/policy.rs:76` as `#[serde(default)] pub source: String`) gains a new value: `"scope_override"`. The policy engine, when compiling rules, **skips the scope fence for any rule with `source = "scope_override"`**. The `PolicyEditor` UI gets a "Scope override" toggle on a rule row that's disabled by default.

This means a user can write a rule like `Approve all reads under /opt/shared` and mark it as `scope_override`, and the fence yields to it. Without the override, the fence is hard. This is the right shape: defaults are safe, escape hatches are explicit.

### 7.5 What this does NOT solve

- **Bash command scope.** `RunBash { cmd: "cat /etc/passwd" }` is still subject to the bash-prefix policy, not the scope fence. The user writes rules like `Reject("cat /etc/*")` for that.
- **TOCTOU.** Symlink races between resolution and execution are best-effort, not bulletproof. The existing policy layer was never TOCTOU-tight and this doesn't change that.

---

## 8. UI Behavior

### 8.1 NewThreadModal agent card

`src/components/NewThreadModal.tsx` gains a second card stacked above the existing sandbox card. On open, the modal calls `get_agent_configs(machineId)`. If the list is empty or all disabled, the launch button is disabled with a tooltip pointing to `EnvModal` for configuration.

The card auto-selects the first enabled agent. The user can override via a `<select>` or pill row. No "Configure agent…" link is wired in v1 — the pill row is the only control. Per-agent settings UI is Phase 7d.

The card payload sent to `onLaunch` includes the selected `agent_kind`. The parent's `launchThread` puts it on the `ThreadSession` row.

### 8.2 Schema migration

`Machine.agents` field changes from a string array to a JSON array of structured records. Migration is a one-shot at app startup:

```rust
fn migrate_machine_agents(raw: Option<String>) -> String {
    let parsed: Vec<serde_json::Value> = raw
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let migrated: Vec<AgentConfig> = parsed.into_iter().map(|v| {
        if let Some(s) = v.as_str() {
            // Legacy: bare string like "OpenCode"
            let kind = s.to_lowercase();
            let known = matches!(kind.as_str(), "opencode" | "hermes");
            AgentConfig { kind, enabled: known }
        } else if let Some(obj) = v.as_object() {
            AgentConfig {
                kind: obj.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string(),
                enabled: obj.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false),
            }
        } else {
            AgentConfig { kind: "unknown".into(), enabled: false }
        }
    }).collect();

    serde_json::to_string(&migrated).unwrap_or_else(|_| "[]".into())
}
```

The migration is conservative: legacy bare strings for unknown agents (e.g. "Claude Code") become `enabled: false` rows that the UI hides. They're not deleted; the user can re-enable them once a real adapter exists.

### 8.3 Status state machine

`ThreadSession.status` gains four values:

| Value | Meaning | Set by |
|---|---|---|
| `idle` | No agent turn in flight; awaiting directive | backend, on TurnComplete |
| `running` | Agent turn in flight | backend, on first AgentEvent of a turn |
| `pending_approval` | An intercept is awaiting user decision | existing |
| `spawning` | Agent process is being launched | frontend (optimistic) → backend confirms |
| `installing` | Install script is running | frontend (optimistic) → backend confirms |
| `error` | Session is in a failed state; user action required | backend, on spawn/process death |

The frontend sets the initial `spawning` / `installing` state on `launchThread` *before* invoking `agent_start`, so the status bar updates in the same React frame as the click. The backend emits a `thread_status_changed` event when it confirms (or corrects to `error`).

The status bar in `SupervisorPlane.tsx:308-321` extends the status map:

```typescript
const statusMap: Record<string, string> = {
  idle: "Thread Idle • Awaiting Directive",
  running: "Agent Running • Supervisor Active",
  pending_approval: "Pending Supervisor Approval",
  spawning: "Spawning Agent…",
  installing: "Installing Agent…",
  error: "Agent Error • Action Required",
};
```

### 8.4 Steering

`SupervisorPlane.tsx:209-228` (the input row) becomes a stateful component:

- `thread.status === "running"`: Send button is replaced by a Stop button (red, square icon, `onClick → agent_cancel`).
- `thread.status === "idle" | "spawning" | "installing" | "error"`: Send button as today.
- `thread.status === "pending_approval"`: button is disabled with text "Resolve pending action first".

Typing in the input and pressing Enter during `running` is an **implicit cancel + redirect**: `App.tsx:sendDirective` first calls `agent_cancel`, awaits the cancellation confirmation (idempotent — returns `Ok` even if the turn already completed), then calls `agent_prompt` with the new text.

On cancel, pending intercepts for the session are drained: each `PendingIntercept` has its oneshot fired with `Resolution::Reject { feedback: "session cancelled" }`. This reuses the same drain path as `agent_restart`.

Partial text from the cancelled turn stays in the stream. An info event is appended: `"[cancelled by user]"`. The status bar flips to `idle`.

### 8.5 Turn complete

When the per-turn channel closes (after `TurnComplete`), the frontend:

1. Appends an info event: `"Turn complete. N actions in T s."` (counts come from the runtime's per-turn tally; we add a counter on the `AgentSession` or sum events on the frontend — frontend is simpler, the events are already there).
2. Flips status to `idle`.
3. Refocuses the input.

No summary card, no suggestion chips, no "what's next" affordance. The intercept cards from the turn stay in the stream as the audit trail.

### 8.6 Inspector auto-open

The `handleInspectContext` function in `App.tsx:393-402` is called by:

- The existing `Inspect Context` button on intercept cards (unchanged).
- Sidebar working-memory entries (NEW).
- **The agent event handler** (NEW, conditional).

The auto-open rule:

```typescript
const handleAgentEvent = (threadId: string, event: AgentEvent) => {
  if (event.kind === "tool_call" && event.action === "read") {
    const lastEventAt = lastStreamEventAt[threadId] ?? 0;
    const now = Date.now();
    const isFirstOrAfterPause = !inspectedFile || (now - lastEventAt > 5000);
    if (isFirstOrAfterPause) {
      handleInspectContext(event.target);
    } else if (inspectedFile) {
      // Inspector is already open; update it to the new file
      handleInspectContext(event.target);
    }
    // else: inspector was dismissed; respect the dismissal
  }
  // ... other event types ...
  lastStreamEventAt[threadId] = Date.now();
};
```

Writes never trigger auto-open. Re-reads after writes *do* trigger auto-open (post-write inspection is the natural follow-up).

---

## 9. Error Model

### 9.1 Per-action errors (typed)

Today, `Err("Failed to provision worktree sandbox: ...")` is a free-form string. We add a typed error enum on the Tauri command return type:

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

The frontend maps each variant to a small set of recovery chips:

| Variant | Recovery chip |
|---|---|
| `Network` | `[Retry]` |
| `Permission` | `[Inspect as supervisor]` |
| `NotFound` | `[Open file tree]` |
| `Policy` | `[Edit policy]` |
| `Internal` | `[Copy error]` |

The Tauri commands that return this are: `add_thread_session`, `request_action`, `sftp_*`, `test_ssh_connection`. Existing free-form `Err` returns are migrated incrementally — the rule is "every new error path returns `ActionError`."

### 9.2 Per-turn errors (`AgentEvent::Error`)

The agent emits structured errors. The frontend renders them as a distinct `StreamEvent.type = "agent_error"` (new variant), styled with the design-system ruby accent (`AGENTS.md:29`). If `recoverable: true`, the input stays enabled with a hint: "Agent reported a recoverable error. You can retry or redirect." If `recoverable: false`, the input is disabled and a banner appears: "Agent stopped: <reason>. [Restart thread] [Pick different agent]."

### 9.3 Per-session errors (watchdog)

The `AgentTransport::try_wait` is polled by a watchdog task. When the underlying process exits (or the SSH channel closes), the watchdog:

1. Sets `ThreadSession.status = "error"` with a reason.
2. Drains all `PendingIntercept` entries for the session with `Resolution::Reject { feedback: "session cancelled" }`.
3. Emits `thread_status_changed { thread_id, status: "error" }`.

The frontend renders the `error` state in the status bar and offers two actions:

- **Restart thread**: same agent, same worktree. Kills the session, clears working memory (§10.4), flips status to `idle`, ready for a new directive.
- **Pick different agent**: re-opens the agent picker; on selection, kills the session, flips status to `spawning`, starts the new agent.

---

## 10. Working Memory

### 10.1 Data model

New table in `db.rs`:

```sql
CREATE TABLE thread_working_memory (
    thread_id      TEXT NOT NULL,
    file_path      TEXT NOT NULL,
    line_count     INTEGER,
    size_bytes     INTEGER,
    modified_at    INTEGER,           -- unix seconds, from SFTP get_metadata
    first_read_at  INTEGER NOT NULL,
    last_read_at   INTEGER NOT NULL,
    PRIMARY KEY (thread_id, file_path),
    FOREIGN KEY (thread_id) REFERENCES thread_sessions(id) ON DELETE CASCADE
);

CREATE INDEX idx_twm_thread_last_read ON thread_working_memory(thread_id, last_read_at DESC);
```

Domain model in `domain/models.rs`:

```rust
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct WorkingMemoryEntry {
    pub file_path: String,
    pub line_count: Option<u32>,
    pub size_bytes: Option<u64>,
    pub modified_at: Option<i64>,
    pub first_read_at: i64,
    pub last_read_at: i64,
}
```

### 10.2 Port methods

`DatabasePort` (in `ports/db.rs`) gains:

```rust
fn upsert_working_memory_entry(
    &self,
    thread_id: &str,
    entry: WorkingMemoryEntry,
) -> Result<(), String>;

fn get_working_memory(
    &self,
    thread_id: &str,
) -> Result<Vec<WorkingMemoryEntry>, String>;

fn clear_working_memory(
    &self,
    thread_id: &str,
) -> Result<(), String>;
```

### 10.3 Population

In `tool_bridge.rs`, when an `fs/read_text_file` is approved (by policy or by user) and the file is read successfully, the bridge:

1. Fetches metadata via `ExecutionPort::get_metadata(machine_id, path)` (already a port method).
2. Constructs a `WorkingMemoryEntry { file_path, line_count, size_bytes, modified_at, first_read_at: now, last_read_at: now }`.
3. Calls `db.upsert_working_memory_entry(thread_id, entry)`.
4. Loads `get_working_memory(thread_id)`; if `len > 20`, deletes the oldest entries until `len == 20`.

The cap-of-20 is enforced in Rust (lazy, one extra read per upsert). The SQL has no cap — the application layer owns the policy.

### 10.4 Restart and delete behavior

- **Thread delete** (`ON DELETE CASCADE`): the working memory rows go with the thread.
- **Thread restart** (§9.3): `db.clear_working_memory(thread_id)` is called. The new agent session has no context and shouldn't see stale "I read this file" entries.
- **Thread switch** (user changes `activeThreadId`): the existing `useEffect` in `App.tsx:182-185` already clears the in-memory `workingMemory` state. The DB rows remain; on re-selection, the frontend reloads them via `get_working_memory`.

### 10.5 UI

`Sidebar.tsx`'s `workingMemory` section is wired to the new Tauri commands. Each entry renders as a clickable row with the file path and the line count / size / relative-modified-time. Clicking calls `handleInspectContext(entry.file_path)` to open the Code Inspector.

The "4.2K TKNS" indicator at the top of the section is **stayed as a stub** in v1 — it shows "—" or "idle" rather than fake token counts. Wiring it to the `Usage` event is a Phase 7f polish item.

---

## 11. Tauri Command Surface

The full list of new and modified Tauri commands. All commands return `Result<T, String>` where `T` is the typed success payload; failures use the new `ActionError` shape where applicable.

### New commands

```rust
// Phase 7a
#[tauri::command]
async fn get_agent_configs(
    state: tauri::State<'_, DatabaseState>,
    machine_id: String,
) -> Result<Vec<AgentConfig>, String>;

#[tauri::command]
async fn get_working_memory(
    state: tauri::State<'_, DatabaseState>,
    thread_id: String,
) -> Result<Vec<WorkingMemoryEntry>, String>;

// Phase 7c
#[tauri::command]
async fn agent_start(
    state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
    agent_kind: String,
) -> Result<AgentStartResult, String>;

#[tauri::command]
async fn agent_install_and_start(
    state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
    agent_kind: String,
) -> Result<Channel<AgentEvent>, String>;

#[tauri::command]
async fn agent_prompt(
    state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
    text: String,
) -> Result<Channel<AgentEvent>, String>;

#[tauri::command]
async fn agent_cancel(
    state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
) -> Result<(), String>;

#[tauri::command]
async fn agent_restart(
    state: tauri::State<'_, AgentRegistryState>,
    thread_id: String,
) -> Result<(), String>;
```

### Modified commands

- `add_thread_session` — accepts and persists the new `agent_kind: Option<String>` field.
- `request_action` — accepts an optional `tool_call_id: Option<String>` parameter. When set, the result is returned both via the global `command_executed` event (for the UI) and as a `tool_call/update` back to the agent session.
- `delete_thread_session` — `ON DELETE CASCADE` handles working memory cleanup; no code change needed beyond the schema migration.
- All commands that return `Err(String)` on the policy / SFTP / worktree paths — migrate to `Err(ActionError)` incrementally.

### New event payloads

```rust
pub const EVENT_THREAD_STATUS_CHANGED: &str = "thread_status_changed";

#[derive(Serialize, Clone)]
pub struct ThreadStatusChanged {
    pub thread_id: String,
    pub status: String,         // "spawning" | "installing" | "running" | "idle" | "error"
    pub reason: Option<String>, // present when status == "error"
}
```

---

## 12. File Layout (Concretely)

The files touched or created by v1, organized by phase. Existing files modified are marked with `(modified)`; new files are marked with `(new)`.

### Phase 7a — Port + domain skeleton

```
src-tauri/src/domain/
  agent_event.rs                  (new)    AgentEvent, ToolCallStatus, PlanEntry, StopReason
  intercept.rs                    (modified) add tool_call_id: Option<String> to InterceptPayload
  policy.rs                       (modified) add source = "scope_override" semantics to PolicyRule docs
  policy_engine.rs                (modified) add ScopeFence pre-rule
  models.rs                       (modified) add WorkingMemoryEntry

src-tauri/src/ports/
  db.rs                           (modified) add 3 working_memory methods; update Machine.agents parsing
  execution.rs                    (modified) add spawn_interactive + InteractiveHandle trait
  agent_runtime.rs                (new)    AgentRuntime, AgentSession, AgentContext, AgentStartError
  agent_execution.rs              (modified) doc comment: "InterceptPayload now carries tool_call_id"

src-tauri/src/adapters/database/
  sqlite.rs                       (modified) implement 3 new methods; add thread_working_memory migration

src-tauri/Cargo.toml              (modified) add tokio-stream, thiserror (if not already), agent-client-protocol (Phase 7b)

src/
  types.ts                        (modified) add AgentEvent, AgentKind, AgentConfig, WorkingMemoryEntry
```

### Phase 7b — AcpRuntime

```
src-tauri/src/adapters/agent/
  mod.rs                          (modified) declare registry, acp, opencode, hermes submodules
  registry.rs                     (new)    AgentRegistry: spawn(kind, ctx) -> Arc<dyn AgentSession>
  acp/
    mod.rs                        (new)
    runtime.rs                    (new)    AcpRuntime: spawns agent, owns transport, drives ACP session
    event_mapper.rs               (new)    ACP notifications -> AgentEvent
    tool_bridge.rs                (new)    ACP fs/* + terminal/* -> PolicyEnforcedExecutionPort
    install.rs                    (new)    run_official_install() over local or SSH transport
  opencode/
    mod.rs                        (new)    AgentConfig + availability check; delegates to AcpRuntime
  hermes/
    mod.rs                        (new)    same shape
```

### Phase 7c — UI wire-up

```
src-tauri/src/
  lib.rs                          (modified) construct AgentRegistry; register new Tauri commands

src/
  App.tsx                         (modified) useAgentSession hook; sendDirective -> agent_prompt
  components/
    NewThreadModal.tsx            (modified) add agent selection card
    SupervisorPlane.tsx           (modified) Stop button; auto-inspector; agent_error event type
    EnvModal.tsx                  (modified) agents[] renders as structured AgentConfig[]
  agentSessionRegistry.ts         (new)    per-thread session metadata (mirrors sessionRegistry.ts)
```

### Phase 7d — Per-agent settings (after v1 ships)

```
src-tauri/src/
  domain/models.rs                (modified) add AgentConfig struct (kind, model, work_dir, env_refs, ...)
src/
  types.ts                        (modified) add full AgentConfig type
src/components/
  AgentConfigEditor.tsx           (new)    per-agent settings modal
  EnvModal.tsx                    (modified) link from "Enabled Agents" to AgentConfigEditor
```

### Phase 7e — Second transport (after v1 ships)

```
src-tauri/src/adapters/agent/
  http/                           (new)    HttpRuntime for non-ACP agents
  registry.rs                     (modified) pick runtime by AgentConfig.transport
```

---

## 13. Phase Plan with Verification

Each phase has a "Done means…" statement. Phases are sequential; don't start the next until the current is verified.

### Phase 7a — Port + domain skeleton

**Scope**: introduce the `AgentRuntime` trait, the `AgentEvent` enum, the `ScopeFence`, the new DB methods, the `spawn_interactive` port method. No UI changes, no actual agent spawns.

**Done means**:
- `cargo build` passes; `cargo test` passes; existing tests in `policy_decorator.rs` and `policy_engine.rs` still pass.
- The SQLite migration adds the `thread_working_memory` table and the index; existing test fixtures still work.
- The `ScopeFence` has unit tests covering: absolute paths inside scope (allow), absolute paths outside (reject), relative paths with `..` (reject), symlinks pointing out (reject), bash actions (defer to prefix policy).
- A `NoopRuntime` is registered in `AgentRegistry` so the wiring compiles and the `agent_start` command returns a structured `AgentStartError::NotFound("noop")` instead of crashing.

### Phase 7b — AcpRuntime

**Scope**: implement the ACP client. Add `acp_agent_client_protocol` (or hand-rolled JSON-RPC). Map `session/update` and `tool_call` to `AgentEvent`. Implement `fs/*` and `terminal/*` client methods.

**Done means**:
- `cargo test` includes an integration test that spawns a mock ACP agent (a small Rust binary that emits canned `session/update` notifications) and verifies the runtime produces the expected `AgentEvent` stream.
- The `tool_bridge` unit tests cover: `fs/read_text_file` happy path (returns file content), `fs/read_text_file` blocked by scope fence (returns Failed with reason), `fs/write_text_file` blocked by user policy (returns Failed, agent sees the rejection as a tool result).
- Manual smoke: launch Hermes locally, send a directive, see the text stream and a tool call come through the per-turn channel.

### Phase 7c — UI wire-up

**Scope**: Tauri commands, modal card, supervisor plane, sidebar working memory, status state machine, error model, inspector auto-open.

**Done means**:
- A new thread can be launched with an agent selected, the agent spawns, a directive is sent, text streams in, a tool call is intercepted, the user approves, the agent continues, the turn completes.
- A thread can be restarted; the working memory is cleared.
- A thread with no agents enabled blocks the launch with a clear error.
- Cancelling a running turn drains the pending intercepts and flips the status to `idle`.
- The first `Read` of a turn auto-opens the inspector; subsequent reads update it; a dismissed inspector does not re-open.
- The `4.2K TKNS` indicator is a stub showing "—" or "idle."
- All existing tests still pass; new Tauri commands are wired in `lib.rs` and `invoke_handler!`.

### Phase 7d — Per-agent settings (post-v1)

**Scope**: structured `AgentConfig` with model, work_dir, env_refs. Settings UI in `EnvModal`.

**Done means**:
- The user can pick the model for opencode and Hermes per machine.
- A working-dir override per agent works (the agent's `cwd` is the override, not the worktree).
- The thread picker remembers the last-used `agent_kind` per machine.

### Phase 7e — Second transport (post-v1)

**Scope**: add a non-ACP runtime (HTTP/Server for whatever the next agent is). The pick of which agent is *based on demand* — we don't pre-commit.

**Done means**:
- The second runtime uses a different IPC pattern (HTTP + SSE, not stdio JSON-RPC).
- The `AcpRuntime` and the new runtime coexist; the dispatcher is unaware of which is in use.
- A non-ACP agent can be configured and used in a thread, end-to-end.

---

## 14. Open Questions for Future Phases

Captured here so they don't get lost, but explicitly **not v1 scope**:

1. **Transcript persistence + context replay** (Phase 8): persist every prompt + every `AgentEvent` to a `thread_messages` table. Resume = new agent session + replay the transcript as a single initial context block. Enables real resume across restarts and lets the user swap agents mid-thread.
2. **Secret management** (Phase 8): the keyring-backed `AgentSecretStore` design from the design interview. Deferred because the user pre-configures the agent for v1.
3. **WASM policy plugins** (Phase 8+): the `ARCHITECTURE.md` plugin host. The scope fence + `PolicyEnforcedExecutionPort` cover v1's needs.
4. **Token/cost usage dashboard** (Phase 7f polish): wire the "4.2K TKNS" indicator to the `Usage` event. Add a per-thread cost summary in the sidebar.
5. **Turn-summary card** (Phase 7f polish): the "what changed in this turn" summary. Deferred because the per-intercept cards are the actionable unit in v1.
6. **Crash report collection / telemetry** (Phase 8): out of scope for v1.
7. **Auto-restart on transient errors** (Phase 8): single restart on user request only in v1.
8. **Plan card visual design** (Phase 7f polish): v1 renders `Plan` events as a text block; a proper card with stepper UI is a polish item.
