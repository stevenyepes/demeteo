# Demeteo Reliability Plan: DAG / Pipeline / SSH

> **Scope:** Improvements to the `StepExecutor` pipeline and the SSH
> transport that materially reduce silent-failure modes, lost work on
> crash / network drop, and accumulated state drift. Source of truth for
> v1 reliability work; cross-references [`DECISIONS.md`](DECISIONS.md) for the
> locked decisions and [`ARCHITECTURE.md`](ARCHITECTURE.md) for transport-level
> invariants.
>
> **Status:** Items marked **[Shipped]** have landed in the codebase and are
> now part of the active behavior — see the `Where` anchors for the
> current path. Items marked **[Open]** are still on the backlog.
>
> **File-line anchors** point at the current `main` of the backend refactor
> branch. Re-verify with `git grep` before implementing; the line numbers
> will drift as surrounding code changes.

---

## 0. Invariants this plan must preserve

1. **Strict serial per project** — at most one running feature per
   project at a time (see [`DECISIONS.md`](DECISIONS.md) decision 18).
2. **Per-step checkpoints are atomic** — a step is "complete" only when
   its artifact is written and (if it's a gate) its decision is recorded
   (see [`DDD_MODEL.md`](DDD_MODEL.md) §4 Feature Orchestration).
3. **Cost and duration are computed at step completion, not estimated
   mid-step** (see [`DECISIONS.md`](DECISIONS.md) decision 15).
4. **Step transitions are the UI contract, not agent transcripts**
   (see [`DDD_MODEL.md`](DDD_MODEL.md) §6 Agent Runtime).
5. **Keyring + ssh-agent for credentials, never plaintext**
   (see [`ARCHITECTURE.md`](ARCHITECTURE.md) §2).

---

## 1. Pipeline / DAG (`src-tauri/src/adapters/step_executor/`)

### P1. Cancel-vs-future distinction — **[Shipped]**

**Where:** [`adapters/agent/event_stream/turn.rs:74-166`](../../src-tauri/src/adapters/agent/event_stream/turn.rs).

The `tokio::select!` body in `stream_agent_turn` now distinguishes the
three terminal outcomes as a typed `TurnResult`:

```rust
pub enum TurnResult {
    Success(TurnOutcome),
    Interrupted,
    Failed(String),
}
```

The cancel arm reads `cancel_watch` (set when the user invokes
`feature_cancel` / `agent_cancel`), calls `session.cancel()`, sets
`run_cancelled = true`, and the post-loop dispatch returns
`TurnResult::Interrupted` instead of being collapsed to the same `None`
that the spawn branch produces. The downstream `match` differentiates
cancel vs spawn-completed, so a step whose session was cancelled is no
longer marked `failed`.

**Verification:** unit tests live under
[`tests/e2e/step_executor.rs`](../../src-tauri/tests/e2e/step_executor.rs).

---

### P2. Atomic state transitions — **[Shipped]**

**Where:** [`adapters/step_executor/driver.rs`](../../src-tauri/src/adapters/step_executor/driver.rs)
plus per-handler sites under
[`adapters/step_executor/steps/`](../../src-tauri/src/adapters/step_executor/steps/)
(agent / gate / parallel / sync).

The driver threads a single `Arc<dyn FeatureRepository>` through every
state transition. Transitions go through `features.step_update(…)`
which writes the `StepExecutionPatch` (status / cost_usd / tokens /
artifact_paths / error_message / iteration_count). The repo adapter
keeps the legacy single-path `artifact_path` column in sync with
`artifact_paths[0]` so older readers keep working.

A crash mid-pipeline leaves `step_executions.status="running"`; the
re-entry scan in
[`adapters/step_executor/driver.rs::maybe_resume`](../../src-tauri/src/adapters/step_executor/driver.rs)
treats that as "not yet started" and re-runs the step from scratch.

---

### P3. Resume from interruption — **[Shipped]**

**Where:** [`adapters/step_executor/driver.rs`](../../src-tauri/src/adapters/step_executor/driver.rs)
and [`ports/step_executor.rs:60-72`](../../src-tauri/src/ports/step_executor.rs)
(`StepExecutor::step_retry` precondition docstring).

`StepStatus::interrupted` is a first-class terminal state used by the
shutdown watchdog. On launch, if any step is in `interrupted`, the
driver inserts a synthetic `GateDecision` row with
`decision = None, feedback = None` and emits `GateRequired` — the user
must clear the gate before the step re-runs.

---

### P4. `parallel` step planner fan-out — **[Shipped]**

**Where:** [`adapters/step_executor/steps/parallel/`](../../src-tauri/src/adapters/step_executor/steps/parallel/)
(mod.rs, planner.rs, subtask.rs, list_unmerged.rs).

The `parallel` step is no longer a stub. `planner.rs` spawns a planner
agent session with a structured-output prompt, parses the response into
`Vec<SubtaskSpec>`, and fans the work out across subtask workers via
`ExecutionPort::spawn_interactive`. `list_unmerged.rs` orders merges
with `MergeExecutor::merge_topological_order` and aggregates results
into the step's artifact.

---

### P5. Conditional edges / `max_iterations` — **[Shipped]**

**Where:** [`adapters/step_executor/driver/failure.rs`](../../src-tauri/src/adapters/step_executor/driver/failure.rs)
and [`adapters/step_executor/driver/verifier.rs`](../../src-tauri/src/adapters/step_executor/driver/verifier.rs).

`StepOutcome` now has `Goto(usize)`, `Loop`, and `Stop` variants.
`StepConfig::on_failure_step_id` and `StepConfig::max_iterations` are
honored end-to-end. Per-step `iteration_count` is tracked on the
`StepExecution` row via `StepExecutionPatch::iteration_count` and the
per-run override flows from
`Feature::loop_iterations` → `ProjectSettings::default_loop_iterations`
→ the engine default (`DEFAULT_LOOP_ITERATIONS = 3`,
[`adapters/step_executor/driver.rs:30`](../../src-tauri/src/adapters/step_executor/driver.rs)).

---

### P6. `accumulated_cost` on the driver — **[Shipped]**

**Where:** [`adapters/step_executor/driver.rs`](../../src-tauri/src/adapters/step_executor/driver.rs).

Per-step cost / duration is written from one place — the driver's
turn-finalize path via `features.step_update(…)`. Per-step handlers no
longer carry an `&mut` accumulator.

---

### P7. Coalesced agent events to UI — **[Shipped]**

**Where:** [`adapters/agent/event_stream/turn.rs`](../../src-tauri/src/adapters/agent/event_stream/turn.rs).

Text deltas are accumulated into a per-turn `text_buffer` inside
`stream_agent_turn` and surfaced only once at turn completion (the
`Artifact` writer reads the buffer). High-frequency `ToolCall` /
`ToolCallUpdate` events still pass through unchanged so the UI's
in-flight indicators stay responsive, but the text stream is no longer
a per-delta IPC storm.

---

## 2. SSH / transport (`src-tauri/src/adapters/ssh/`)

### S1. Stale sessions stay in cache — **[Partial]**

**Where:** [`adapters/ssh/`](../../src-tauri/src/adapters/ssh/).

The session cache evicts on the next probe failure. A half-open
connection (TCP up, SSH dropped) still serves commands that timeout
rather than reconnecting immediately.

**Open follow-up:** track `last_used: AtomicU64` on `SftpSession` and
probe `sess.closed()` before returning a cached session; add a
background reaper for sessions idle > 5 min.

---

### S2. `Sftp` is serialized by a single `Mutex<Sftp>` — **[Partial]**

**Where:** [`adapters/ssh/`](../../src-tauri/src/adapters/ssh/).

`ssh2::Sftp` is not thread-safe; the current mutex-per-session design
serializes SFTP ops for one machine across the whole app.

**Open follow-up:** `tokio::sync::Mutex` and a "never hold across
awaits" lint; longer-term `russh` migration.

---

### S3. `spawn_interactive` validates cwd + binary — **[Shipped]**

**Where:** [`adapters/ssh/`](../../src-tauri/src/adapters/ssh/).

Before exec the spawn path probes the cwd with `test -d <cwd> && echo
OK`, drains stderr for the first 200 ms post-exec, and verifies the
binary resolves on the remote `$PATH` with `command -v <binary>`.
Failures surface as `AgentStartError::SpawnFailed("cwd not found: …")`
or `AgentStartError::SpawnFailed("binary not found: …")`.

---

### S4. Retry on transient SSH drops — **[Open]**

**Where:** [`adapters/ssh/`](../../src-tauri/src/adapters/ssh/).

One dropped network = full pipeline stop. The driver only sees the
subprocess return and marks the step `Failed`.

**Fix:** add a `with_ssh_retry(future, attempts: u32)` wrapper at the
`ExecutionPort` boundary that re-establishes the session on
`Err(SshError::ConnectionLost)` and re-execs the call.

---

### S5. Port-forwarding state not covered by watchdog — **[Open]**

**Where:** [`forward.rs`](../../src-tauri/src/forward.rs).

Listeners aren't torn down when a machine is deleted or the SSH session
drops. Symptom: deleted-machine forwards keep accepting connections
until app restart.

**Fix:** add `ForwardState::prune_for_machine(machine_id)` and call it
from `commands::machine::delete_machine` and on connection-drop in S1.

---

### S6. `RouterExecutionPort` string-based dispatch — **[Audit only]**

**Where:** [`adapters/router.rs`](../../src-tauri/src/adapters/router.rs).

Resolves `auth_type` via `match` with a default branch. Audit
recommended before changing anything; the fix (if needed) is to
return a typed `RouterError::UnknownAuthType(String)` and bubble that
up.

---

## 3. Cross-cutting

### X1. Type the pipeline state — **[Shipped]**

**Where:** [`adapters/step_executor/driver.rs`](../../src-tauri/src/adapters/step_executor/driver.rs).

The driver holds a typed `ExecutionDriver` struct (see lines 47-80) with
explicit fields for `features`, `gates`, `projects`, `merge_executor`,
`registry`, `agent_exec`, `exec`, `artifacts`, `attachments`,
`app_settings`, `git_ops`, `gate_waiters`, `driver_registry`,
`notif`, `signals`. Every transition goes through one of the typed
ports; the `failure.rs` / `verifier.rs` submodules encode the
`StepOutcome` enum and the per-step iteration / max-iterations logic.

---

### X2. `step_executor::tests` integration suite — **[Shipped]**

**Where:** [`tests/e2e/step_executor.rs`](../../src-tauri/tests/e2e/step_executor.rs).

The integration suite relocated from `adapters/step_executor/tests.rs`
per Phase D11. Coverage:
- happy path: 3-step workflow with 1 gate ends `completed`
- cancel during `agent` step → step `cancelled` / feature `cancelled` (P1)
- gate "redirect" → `Goto(target)` advances correctly (P5)
- `max_iterations: 2` stops the loop (P5)
- predecessor-running guard for `step_retry` and `gate_decide` (§7)
- `feature_sync` and `feature_resolve_sync_conflicts` typed outcomes

---

## 4. Suggested sequencing (now historical)

The original sequencing table is preserved for traceability; the items
it lists are all shipped or in progress.

| Order | Items | Status |
|------:|-------|--------|
| 1 | P1 + P6 + X1 | Shipped |
| 2 | P5 | Shipped |
| 3 | P7 | Shipped |
| 4 | S1 + S3 | P3 partial, S3 shipped |
| 5 | P3 | Shipped |
| 6 | X2 | Shipped |
| 7 | P4 | Shipped |
| 8 | S4 | Open |
| 9 | S2 / S5 / S6 | Open |

---

## 5. Done-means per item (verification commands)

```bash
cd src-tauri && cargo test --lib step_executor
cd src-tauri && cargo test --lib ssh
cd src-tauri && cargo test --test e2e
```

Manual smoke:

1. Launch a feature with a 5-step workflow.
2. Kill demeteo mid-step.
3. Relaunch — verify synthetic gate surfaces.
4. SSH to the target, kill `sshd` mid-step — verify retry kicks in.
5. Inspect `demeteo.db` after each: `step_executions` rows must be in a
   single coherent terminal state (no `running` rows older than the
   kill timestamp).

---

## 6. Cross-references

- [`DECISIONS.md`](DECISIONS.md) decisions 14, 15 — feature re-entry, telemetry.
- [`DDD_MODEL.md`](DDD_MODEL.md) §4 Feature Orchestration invariants.
- [`ARCHITECTURE.md`](ARCHITECTURE.md) §2 Port Catalogue (`StepExecutor`, `GatePresenter`).

---

## 7. Predecessor-running guard — **[Shipped]**

> **Origin:** the pipeline view could show a stale `awaiting_gate` chip on
> a step the user already retried, allowing them to click "Retry Step"
> or "Approve Gate" on a step whose predecessor was still in flight.
> Outcome: actions that should have been blocked by backend invariants
> reached `replay_steps_from` / the gate waiter, racing the still-running
> agent.

### Trigger conditions

- A `gate_required` event arrives while a predecessor agent step is
  still `running` / `verifying` (e.g. the agent hadn't yet finalised its
  artifact when the orchestrator fired the gate).
- A `step_retry` IPC lands on a `failed` step whose earlier step is
  stuck in `awaiting_gate` after a system restart (the watchdog
  re-emits the gate; the executor never resumes because the user never
  acts on it).

### What the guard does

A single helper invoked from
[`ports/step_executor.rs:60-72`](../../src-tauri/src/ports/step_executor.rs)
(`StepExecutor::step_retry` precondition docstring) and
[`ports/step_executor.rs:152-165`](../../src-tauri/src/ports/step_executor.rs)
(`GatePresenter::gate_decide` precondition docstring) walks
`steps_for_feature(target.feature_id)` and returns
`Err(AppError::validation)` on the first non-terminal predecessor with
`step_index < target.step_index`. The four blocking statuses are
`pending`, `running`, `verifying`, `awaiting_gate`; `completed`,
`failed`, `interrupted`, `skipped` are non-blocking.

The returned `AppError::validation` carries a message of the form:

> `Step '<name>' is still <status>; wait for it to finish before <intent>.`

so the UI can both render the blocker by name and route the toast to a
warning instead of an error.

### UI blocking contract (defence in depth)

The frontend mirrors the same rule in pure TypeScript via
`findActivePredecessor` in
[`src/lib/features.ts`](../../src/lib/features.ts). Two surfaces use it:

- `FeatureDetail.tsx`: each failed/interrupted step card computes
  `activePredecessor`. When non-null, the "Retry Step" button is
  `disabled`, a rose-bordered banner names the blocker, and the
  toast routes to `kind: 'warning'` via `isBlockingError`.
- `GateView.tsx`: on mount the modal calls `step_list_for_run`,
  computes `blockedBy`, and renders a persistent banner above the
  Approve / Redirect buttons (both disabled). The "Abort feature"
  button stays enabled — aborting is a separate intent.

A `gate_required` / `step_progress` event triggers an immediate
`loadFeatureData()` so the stale "active" chip clears on the next tick
instead of waiting for the 1 Hz heartbeat.

### Tests

The integration suite in
[`tests/e2e/step_executor.rs`](../../src-tauri/tests/e2e/step_executor.rs)
covers:

- `step_retry` blocked by an active predecessor returns
  `AppError::Validation` naming the blocker.
- `gate_decide` blocked by an active predecessor returns
  `AppError::Validation` naming the blocker.
- `step_retry` unblocks when the predecessor is terminal
  (`completed` / `skipped` / `failed`).
- The helper reports the *earliest* non-terminal predecessor (lower
  `step_index` wins).